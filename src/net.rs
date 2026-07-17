//! Network interface information from /sys/class/net.
//!
//! Shows network interfaces with their MAC addresses, MTU, operational state,
//! statistics, IP addresses, and wireless signal info.
//!
//! IP addresses are parsed from /proc/net/fib_trie (IPv4) and /proc/net/if_inet6 (IPv6).
//! Wireless signal quality comes from /proc/net/wireless.

use crate::cli::GlobalOptions;
use crate::fields::net as f;
use crate::filter::{matches_any, opt_str};
use crate::io;
use crate::json::{begin_kv_output_streaming, StreamingJsonWriter};
use crate::print::{self, TextWriter};
use crate::stack::StackString;

const NET_SYSFS_PATH: &str = "/sys/class/net";
const PROC_NET_WIRELESS: &str = "/proc/net/wireless";
const PROC_NET_IF_INET6: &str = "/proc/net/if_inet6";
const PROC_NET_FIB_TRIE: &str = "/proc/net/fib_trie";
const PROC_NET_ROUTE: &str = "/proc/net/route";

// =============================================================================
// Stack-based lookup tables and limits
// =============================================================================

/// Maximum number of interfaces to track.
const MAX_INTERFACES: usize = 64;

/// Maximum number of IPs per interface.
const MAX_IPS_PER_INTERFACE: usize = 16;

/// Maximum total IPs to track across all interfaces.
const MAX_TOTAL_IPS: usize = 256;

/// Maximum number of routes to track.
const MAX_ROUTES: usize = 256;

/// Wireless signal information.
#[derive(Clone, Copy)]
pub struct WirelessInfo {
    /// Link quality (0-100 typically)
    pub link_quality: i32,
    /// Signal level in dBm
    pub signal_dbm: i32,
    /// Noise level in dBm (often -256 meaning unavailable)
    pub noise_dbm: i32,
}

/// Stack-based IP address list for an interface.
struct IpList {
    ips: [StackString<64>; MAX_IPS_PER_INTERFACE],
    count: usize,
}

impl IpList {
    fn new() -> Self {
        Self {
            ips: core::array::from_fn(|_| StackString::new()),
            count: 0,
        }
    }

    fn push(&mut self, ip: &str) {
        if self.count < MAX_IPS_PER_INTERFACE {
            self.ips[self.count] = StackString::from_str(ip);
            self.count += 1;
        }
    }

    fn is_empty(&self) -> bool {
        self.count == 0
    }

    fn first(&self) -> Option<&str> {
        if self.count > 0 {
            Some(self.ips[0].as_str())
        } else {
            None
        }
    }
}

/// Stack-based IPv4 address map.
struct Ipv4Map {
    entries: [(StackString<16>, IpList); MAX_INTERFACES],
    count: usize,
}

impl Ipv4Map {
    fn new() -> Self {
        Self {
            entries: core::array::from_fn(|_| (StackString::new(), IpList::new())),
            count: 0,
        }
    }

    fn get_or_insert(&mut self, iface: &str) -> Option<&mut IpList> {
        // Check if already exists
        for i in 0..self.count {
            if self.entries[i].0.as_str() == iface {
                return Some(&mut self.entries[i].1);
            }
        }
        // Add new entry if space
        if self.count < MAX_INTERFACES {
            self.entries[self.count].0 = StackString::from_str(iface);
            self.count += 1;
            return Some(&mut self.entries[self.count - 1].1);
        }
        None
    }

    fn get(&self, iface: &str) -> Option<&IpList> {
        for i in 0..self.count {
            if self.entries[i].0.as_str() == iface {
                return Some(&self.entries[i].1);
            }
        }
        None
    }
}

/// Stack-based IPv6 address map.
struct Ipv6Map {
    entries: [(StackString<16>, IpList); MAX_INTERFACES],
    count: usize,
}

impl Ipv6Map {
    fn new() -> Self {
        Self {
            entries: core::array::from_fn(|_| (StackString::new(), IpList::new())),
            count: 0,
        }
    }

    fn get_or_insert(&mut self, iface: &str) -> Option<&mut IpList> {
        for i in 0..self.count {
            if self.entries[i].0.as_str() == iface {
                return Some(&mut self.entries[i].1);
            }
        }
        if self.count < MAX_INTERFACES {
            self.entries[self.count].0 = StackString::from_str(iface);
            self.count += 1;
            return Some(&mut self.entries[self.count - 1].1);
        }
        None
    }

    fn get(&self, iface: &str) -> Option<&IpList> {
        for i in 0..self.count {
            if self.entries[i].0.as_str() == iface {
                return Some(&self.entries[i].1);
            }
        }
        None
    }
}

/// Stack-based wireless info map.
struct WirelessMap {
    entries: [(StackString<16>, WirelessInfo); MAX_INTERFACES],
    count: usize,
}

impl WirelessMap {
    fn new() -> Self {
        Self {
            entries: core::array::from_fn(|_| (StackString::new(), WirelessInfo { link_quality: 0, signal_dbm: 0, noise_dbm: 0 })),
            count: 0,
        }
    }

    fn insert(&mut self, iface: &str, info: WirelessInfo) {
        if self.count < MAX_INTERFACES {
            self.entries[self.count].0 = StackString::from_str(iface);
            self.entries[self.count].1 = info;
            self.count += 1;
        }
    }

    fn get(&self, iface: &str) -> Option<&WirelessInfo> {
        for i in 0..self.count {
            if self.entries[i].0.as_str() == iface {
                return Some(&self.entries[i].1);
            }
        }
        None
    }
}

/// Stack-based route table.
struct RouteTable {
    entries: [(StackString<16>, u32, u32); MAX_ROUTES],
    count: usize,
}

impl RouteTable {
    fn new() -> Self {
        Self {
            entries: core::array::from_fn(|_| (StackString::new(), 0, 0)),
            count: 0,
        }
    }

    fn push(&mut self, iface: &str, dest: u32, mask: u32) {
        if self.count < MAX_ROUTES {
            self.entries[self.count].0 = StackString::from_str(iface);
            self.entries[self.count].1 = dest;
            self.entries[self.count].2 = mask;
            self.count += 1;
        }
    }
}

/// Stack-based set for IP deduplication.
struct IpSet {
    ips: [StackString<32>; MAX_TOTAL_IPS],
    count: usize,
}

impl IpSet {
    fn new() -> Self {
        Self {
            ips: core::array::from_fn(|_| StackString::new()),
            count: 0,
        }
    }

    fn contains(&self, ip: &str) -> bool {
        for i in 0..self.count {
            if self.ips[i].as_str() == ip {
                return true;
            }
        }
        false
    }

    fn insert(&mut self, ip: &str) -> bool {
        if self.contains(ip) {
            return false;
        }
        if self.count < MAX_TOTAL_IPS {
            self.ips[self.count] = StackString::from_str(ip);
            self.count += 1;
            return true;
        }
        false
    }
}

/// Information about a single network interface.
pub struct NetInterface {
    /// Interface name (e.g., "eth0", "wlan0")
    pub name: StackString<16>,
    /// MAC address (e.g., "00:11:22:33:44:55")
    pub mac_address: Option<StackString<32>>,
    /// MTU (Maximum Transmission Unit)
    pub mtu: Option<u32>,
    /// Operational state (up, down, unknown, etc.)
    pub operstate: Option<StackString<16>>,
    /// Link speed in Mbps
    pub speed_mbps: Option<u32>,
    /// Interface type
    pub if_type: Option<u16>,
    /// TX queue length
    pub tx_queue_len: Option<u32>,
    /// Is the interface up (carrier detected)?
    pub carrier: Option<bool>,
    /// Duplex mode (full, half)
    pub duplex: Option<StackString<16>>,
    /// Wireless info
    pub wireless: Option<WirelessInfo>,
    /// Bytes received
    pub rx_bytes: Option<u64>,
    /// Bytes transmitted
    pub tx_bytes: Option<u64>,
    /// Packets received
    pub rx_packets: Option<u64>,
    /// Packets transmitted
    pub tx_packets: Option<u64>,
    /// Receive errors
    pub rx_errors: Option<u64>,
    /// Transmit errors
    pub tx_errors: Option<u64>,
    /// Receive dropped
    pub rx_dropped: Option<u64>,
    /// Transmit dropped
    pub tx_dropped: Option<u64>,
}

impl NetInterface {
    /// Read interface with pre-parsed IP and wireless data.
    fn read_with_extra(
        name: &str,
        _ipv4_map: &Ipv4Map,
        _ipv6_map: &Ipv6Map,
        wireless_map: &WirelessMap,
    ) -> Option<Self> {
        let base: StackString<64> = io::join_path(NET_SYSFS_PATH, name);

        if !io::path_exists(base.as_str()) {
            return None;
        }

        let addr_path: StackString<128> = io::join_path(base.as_str(), "address");
        let mtu_path: StackString<128> = io::join_path(base.as_str(), "mtu");
        let oper_path: StackString<128> = io::join_path(base.as_str(), "operstate");
        let speed_path: StackString<128> = io::join_path(base.as_str(), "speed");
        let type_path: StackString<128> = io::join_path(base.as_str(), "type");
        let txq_path: StackString<128> = io::join_path(base.as_str(), "tx_queue_len");
        let carrier_path: StackString<128> = io::join_path(base.as_str(), "carrier");
        let duplex_path: StackString<128> = io::join_path(base.as_str(), "duplex");

        let stats_base: StackString<128> = io::join_path(base.as_str(), "statistics");
        let rx_bytes_path: StackString<128> = io::join_path(stats_base.as_str(), "rx_bytes");
        let tx_bytes_path: StackString<128> = io::join_path(stats_base.as_str(), "tx_bytes");
        let rx_packets_path: StackString<128> = io::join_path(stats_base.as_str(), "rx_packets");
        let tx_packets_path: StackString<128> = io::join_path(stats_base.as_str(), "tx_packets");
        let rx_errors_path: StackString<128> = io::join_path(stats_base.as_str(), "rx_errors");
        let tx_errors_path: StackString<128> = io::join_path(stats_base.as_str(), "tx_errors");
        let rx_dropped_path: StackString<128> = io::join_path(stats_base.as_str(), "rx_dropped");
        let tx_dropped_path: StackString<128> = io::join_path(stats_base.as_str(), "tx_dropped");

        Some(NetInterface {
            name: StackString::from_str(name),
            mac_address: io::read_file_stack(addr_path.as_str()),
            mtu: io::read_file_parse(mtu_path.as_str()),
            operstate: io::read_file_stack(oper_path.as_str()),
            speed_mbps: io::read_file_parse(speed_path.as_str()),
            if_type: io::read_file_parse(type_path.as_str()),
            tx_queue_len: io::read_file_parse(txq_path.as_str()),
            carrier: io::read_file_parse::<u8>(carrier_path.as_str()).map(|v| v != 0),
            duplex: io::read_file_stack(duplex_path.as_str()),
            wireless: wireless_map.get(name).copied(),
            rx_bytes: io::read_file_parse(rx_bytes_path.as_str()),
            tx_bytes: io::read_file_parse(tx_bytes_path.as_str()),
            rx_packets: io::read_file_parse(rx_packets_path.as_str()),
            tx_packets: io::read_file_parse(tx_packets_path.as_str()),
            rx_errors: io::read_file_parse(rx_errors_path.as_str()),
            tx_errors: io::read_file_parse(tx_errors_path.as_str()),
            rx_dropped: io::read_file_parse(rx_dropped_path.as_str()),
            tx_dropped: io::read_file_parse(tx_dropped_path.as_str()),
        })
    }

    /// Check if this interface matches the filter pattern.
    fn matches_filter(&self, pattern: &str, case_insensitive: bool) -> bool {
        let fields = [
            self.name.as_str(),
            opt_str(&self.mac_address),
            opt_str(&self.operstate),
        ];
        matches_any(&fields, pattern, case_insensitive)
    }

    /// Output as text.
    fn print_text(&self, verbose: bool, human: bool, ipv4_map: &Ipv4Map, ipv6_map: &Ipv6Map) {
        let mut w = TextWriter::new();

        w.field_str(f::NAME, self.name.as_str());

        if let Some(ref mac) = self.mac_address {
            w.field_str(f::MAC, mac.as_str());
        }
        if let Some(mtu) = self.mtu {
            w.field_u64(f::MTU, mtu as u64);
        }
        if let Some(ref state) = self.operstate {
            w.field_str(f::STATE, state.as_str());
        }
        if let Some(speed) = self.speed_mbps {
            w.field_u64(f::SPEED, speed as u64);
        }

        // Show first IPv4 address
        if let Some(ip_list) = ipv4_map.get(self.name.as_str()) {
            if let Some(ip) = ip_list.first() {
                w.field_str(f::IP, ip);
            }
        }

        // Show wireless signal if available
        if let Some(ref wifi) = self.wireless {
            let mut signal: StackString<16> = StackString::new();
            let mut buf = itoa::Buffer::new();
            signal.push_str(buf.format(wifi.signal_dbm));
            signal.push_str("dBm");
            w.field_str(f::SIGNAL, signal.as_str());
        }

        if verbose {
            // Show all IPv4 addresses
            if let Some(ip_list) = ipv4_map.get(self.name.as_str()) {
                if ip_list.count > 1 {
                    let mut ips: StackString<256> = StackString::new();
                    for i in 0..ip_list.count {
                        if i > 0 {
                            ips.push(',');
                        }
                        ips.push_str(ip_list.ips[i].as_str());
                    }
                    w.field_str(f::IPV4, ips.as_str());
                }
            }

            // Show IPv6 addresses
            if let Some(ip_list) = ipv6_map.get(self.name.as_str()) {
                if !ip_list.is_empty() {
                    let mut ips: StackString<512> = StackString::new();
                    for i in 0..ip_list.count {
                        if i > 0 {
                            ips.push(',');
                        }
                        ips.push_str(ip_list.ips[i].as_str());
                    }
                    w.field_str(f::IPV6, ips.as_str());
                }
            }

            // Wireless details
            if let Some(ref wifi) = self.wireless {
                w.field_i64(f::LINK, wifi.link_quality as i64);
                if wifi.noise_dbm != -256 {
                    let mut noise: StackString<16> = StackString::new();
                    let mut buf = itoa::Buffer::new();
                    noise.push_str(buf.format(wifi.noise_dbm));
                    noise.push_str("dBm");
                    w.field_str(f::NOISE, noise.as_str());
                }
            }

            if let Some(ref duplex) = self.duplex {
                w.field_str(f::DUPLEX, duplex.as_str());
            }
            if let Some(carrier) = self.carrier {
                w.field_u64(f::CARRIER, if carrier { 1 } else { 0 });
            }
            if human {
                if let Some(rx) = self.rx_bytes {
                    let s = io::format_human_size(rx);
                    w.field_str("rx", s.as_str());
                }
                if let Some(tx) = self.tx_bytes {
                    let s = io::format_human_size(tx);
                    w.field_str("tx", s.as_str());
                }
            } else {
                if let Some(rx) = self.rx_bytes {
                    w.field_u64(f::RX_BYTES, rx);
                }
                if let Some(tx) = self.tx_bytes {
                    w.field_u64(f::TX_BYTES, tx);
                }
            }
            if let Some(rx) = self.rx_packets {
                w.field_u64(f::RX_PACKETS, rx);
            }
            if let Some(tx) = self.tx_packets {
                w.field_u64(f::TX_PACKETS, tx);
            }
            if let Some(v) = self.rx_errors {
                w.field_u64(f::RX_ERRORS, v);
            }
            if let Some(v) = self.tx_errors {
                w.field_u64(f::TX_ERRORS, v);
            }
        }

        w.finish();
    }

    /// Write as JSON object.
    fn write_json(&self, w: &mut StreamingJsonWriter, verbose: bool, human: bool, ipv4_map: &Ipv4Map, ipv6_map: &Ipv6Map) {
        w.array_object_begin();

        w.field_str(f::NAME, self.name.as_str());
        w.field_str_opt(f::MAC, self.mac_address.as_ref().map(|s| s.as_str()));
        w.field_u64_opt(f::MTU, self.mtu.map(|v| v as u64));
        w.field_str_opt(f::STATE, self.operstate.as_ref().map(|s| s.as_str()));
        w.field_u64_opt(f::SPEED, self.speed_mbps.map(|v| v as u64));

        // First IPv4 address
        if let Some(ip_list) = ipv4_map.get(self.name.as_str()) {
            if let Some(ip) = ip_list.first() {
                w.field_str(f::IP, ip);
            }
        }

        // Wireless signal
        if let Some(ref wifi) = self.wireless {
            w.field_i64(f::SIGNAL, wifi.signal_dbm as i64);
        }

        if verbose {
            // All IPv4 addresses
            if let Some(ip_list) = ipv4_map.get(self.name.as_str()) {
                if !ip_list.is_empty() {
                    w.field_array(f::IPV4);
                    for i in 0..ip_list.count {
                        w.array_string(ip_list.ips[i].as_str());
                    }
                    w.end_field_array();
                }
            }

            // All IPv6 addresses
            if let Some(ip_list) = ipv6_map.get(self.name.as_str()) {
                if !ip_list.is_empty() {
                    w.field_array(f::IPV6);
                    for i in 0..ip_list.count {
                        w.array_string(ip_list.ips[i].as_str());
                    }
                    w.end_field_array();
                }
            }

            // Full wireless info
            if let Some(ref wifi) = self.wireless {
                w.field_i64(f::LINK, wifi.link_quality as i64);
                if wifi.noise_dbm != -256 {
                    w.field_i64(f::NOISE, wifi.noise_dbm as i64);
                }
            }

            w.field_str_opt(f::DUPLEX, self.duplex.as_ref().map(|s| s.as_str()));
            w.field_u64_opt("if_type", self.if_type.map(|v| v as u64));
            w.field_u64_opt("tx_queue_len", self.tx_queue_len.map(|v| v as u64));
            if let Some(carrier) = self.carrier {
                w.field_bool(f::CARRIER, carrier);
            }
            if human {
                if let Some(rx) = self.rx_bytes {
                    let s = io::format_human_size(rx);
                    w.field_str("rx", s.as_str());
                }
                if let Some(tx) = self.tx_bytes {
                    let s = io::format_human_size(tx);
                    w.field_str("tx", s.as_str());
                }
            } else {
                w.field_u64_opt(f::RX_BYTES, self.rx_bytes);
                w.field_u64_opt(f::TX_BYTES, self.tx_bytes);
            }
            w.field_u64_opt(f::RX_PACKETS, self.rx_packets);
            w.field_u64_opt(f::TX_PACKETS, self.tx_packets);
            w.field_u64_opt(f::RX_ERRORS, self.rx_errors);
            w.field_u64_opt(f::TX_ERRORS, self.tx_errors);
            w.field_u64_opt(f::RX_DROPPED, self.rx_dropped);
            w.field_u64_opt(f::TX_DROPPED, self.tx_dropped);
        }

        w.array_object_end();
    }
}

/// Parse /proc/net/wireless for signal info.
fn parse_proc_net_wireless(wireless_map: &mut WirelessMap) {
    let content: Option<StackString<4096>> = io::read_file_stack(PROC_NET_WIRELESS);
    let Some(content) = content else { return };

    // Skip header lines (first two lines)
    for line in content.as_str().lines().skip(2) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Format: "wlan0: 0000   70.  -40.  -256."
        let mut parts = line.splitn(2, ':');
        let iface = match parts.next() {
            Some(s) => s.trim(),
            None => continue,
        };
        let rest = match parts.next() {
            Some(s) => s,
            None => continue,
        };

        let values: [&str; 4] = {
            let mut arr = [""; 4];
            for (i, v) in rest.split_whitespace().take(4).enumerate() {
                arr[i] = v;
            }
            arr
        };

        if values[3].is_empty() {
            continue;
        }

        let parse_val = |s: &str| -> i32 {
            s.trim_end_matches('.').parse().unwrap_or(0)
        };

        let link = parse_val(values[1]);
        let level = parse_val(values[2]);
        let noise = parse_val(values[3]);

        wireless_map.insert(iface, WirelessInfo {
            link_quality: link,
            signal_dbm: level,
            noise_dbm: noise,
        });
    }
}

/// Parse /proc/net/if_inet6 for IPv6 addresses.
fn parse_proc_net_if_inet6(ipv6_map: &mut Ipv6Map) {
    let content: Option<StackString<32768>> = io::read_file_stack(PROC_NET_IF_INET6);
    let Some(content) = content else { return };

    for line in content.as_str().lines() {
        let mut parts = line.split_whitespace();
        let addr_hex = match parts.next() { Some(s) => s, None => continue };
        let _ifindex = parts.next();
        let prefix_len = match parts.next() { Some(s) => s, None => continue };
        let _scope = parts.next();
        let _flags = parts.next();
        let ifname = match parts.next() { Some(s) => s, None => continue };

        // Convert hex address to IPv6 notation
        if let Some(addr) = hex_to_ipv6(addr_hex) {
            let mut addr_with_prefix: StackString<64> = StackString::new();
            addr_with_prefix.push_str(addr.as_str());
            addr_with_prefix.push('/');
            addr_with_prefix.push_str(prefix_len);

            if let Some(ip_list) = ipv6_map.get_or_insert(ifname) {
                ip_list.push(addr_with_prefix.as_str());
            }
        }
    }
}

/// Convert 32-char hex string to IPv6 address notation.
fn hex_to_ipv6(hex: &str) -> Option<StackString<64>> {
    if hex.len() != 32 {
        return None;
    }

    let mut s: StackString<64> = StackString::new();
    for i in 0..8 {
        if i > 0 {
            s.push(':');
        }
        s.push_str(&hex[i * 4..(i + 1) * 4]);
    }
    Some(s)
}

/// Parse /proc/net/route to build route table.
fn parse_proc_net_route(routes: &mut RouteTable) {
    let content: Option<StackString<32768>> = io::read_file_stack(PROC_NET_ROUTE);
    let Some(content) = content else { return };

    // Skip header line
    for line in content.as_str().lines().skip(1) {
        let mut parts = line.split('\t');
        let iface = match parts.next() { Some(s) => s, None => continue };
        let dest_hex = match parts.next() { Some(s) => s, None => continue };
        // Skip gateway, flags, refcnt, use, metric
        for _ in 0..5 { parts.next(); }
        let mask_hex = match parts.next() { Some(s) => s, None => continue };

        if let (Some(dest), Some(mask)) = (parse_route_hex(dest_hex), parse_route_hex(mask_hex)) {
            routes.push(iface, dest, mask);
        }
    }
}

/// Parse hex IP from /proc/net/route and normalize to network byte order.
fn parse_route_hex(hex: &str) -> Option<u32> {
    let val = u32::from_str_radix(hex, 16).ok()?;
    // On little-endian, swap bytes to get network order
    #[cfg(target_endian = "little")]
    let val = val.swap_bytes();
    Some(val)
}

/// Find which interface an IP belongs to based on routing table.
fn find_interface_for_ip<'a>(ip: &str, routes: &'a RouteTable) -> Option<&'a str> {
    let mut parts_iter = ip.split('.');
    let p0: u8 = parts_iter.next()?.parse().ok()?;
    let p1: u8 = parts_iter.next()?.parse().ok()?;
    let p2: u8 = parts_iter.next()?.parse().ok()?;
    let p3: u8 = parts_iter.next()?.parse().ok()?;

    let ip_val = u32::from_be_bytes([p0, p1, p2, p3]);

    let mut best_match: Option<(&str, u32)> = None;

    for i in 0..routes.count {
        let (ref iface, dest, mask) = routes.entries[i];
        if (ip_val & mask) == dest {
            match best_match {
                None => best_match = Some((iface.as_str(), mask)),
                Some((_, best_mask)) if mask > best_mask => {
                    best_match = Some((iface.as_str(), mask));
                }
                _ => {}
            }
        }
    }

    best_match.map(|(iface, _)| iface)
}

/// Parse /proc/net/fib_trie to extract local IPv4 addresses.
fn parse_proc_net_fib_trie(ipv4_map: &mut Ipv4Map, routes: &RouteTable) {
    let content: Option<StackString<65536>> = io::read_file_stack(PROC_NET_FIB_TRIE);
    let Some(content) = content else { return };

    let mut seen = IpSet::new();
    let mut current_ip: Option<StackString<32>> = None;

    for line in content.as_str().lines() {
        if seen.count >= MAX_TOTAL_IPS {
            break;
        }

        let trimmed = line.trim();

        // Look for IP address lines: "|-- 192.168.1.100"
        if let Some(ip_part) = trimmed.strip_prefix("|-- ") {
            // Validate it looks like an IP
            if ip_part.chars().all(|c| c.is_ascii_digit() || c == '.') {
                current_ip = Some(StackString::from_str(ip_part));
            }
        } else if trimmed.contains("/32 host LOCAL") {
            if let Some(ref ip) = current_ip {
                if !seen.insert(ip.as_str()) {
                    continue;
                }

                // Loopback addresses (127.x.x.x) go to 'lo'
                if ip.as_str().starts_with("127.") {
                    if let Some(ip_list) = ipv4_map.get_or_insert("lo") {
                        ip_list.push(ip.as_str());
                    }
                } else if let Some(iface) = find_interface_for_ip(ip.as_str(), routes) {
                    if let Some(ip_list) = ipv4_map.get_or_insert(iface) {
                        ip_list.push(ip.as_str());
                    }
                }
            }
        }
    }
}

/// Entry point for `kv net` subcommand.
pub fn run(opts: &GlobalOptions) -> i32 {
    if !io::path_exists(NET_SYSFS_PATH) {
        if opts.json {
            let mut w = begin_kv_output_streaming(opts.pretty, "net");
            w.field_array("data");
            w.end_field_array();
            w.end_object();
            w.finish();
        } else {
            print::println("net: no network interfaces found");
        }
        return 0;
    }

    // Pre-parse all the supplementary data
    let mut wireless_map = WirelessMap::new();
    let mut ipv4_map = Ipv4Map::new();
    let mut ipv6_map = Ipv6Map::new();
    let mut routes = RouteTable::new();

    parse_proc_net_wireless(&mut wireless_map);
    parse_proc_net_if_inet6(&mut ipv6_map);
    parse_proc_net_route(&mut routes);
    parse_proc_net_fib_trie(&mut ipv4_map, &routes);

    let filter = opts.filter.as_ref().map(|s| s.as_str());
    let case_insensitive = opts.filter_case_insensitive;

    if opts.json {
        let mut w = begin_kv_output_streaming(opts.pretty, "net");
        w.field_array("data");

        let mut count = 0;
        io::for_each_dir_entry(NET_SYSFS_PATH, |name| {
            if let Some(iface) = NetInterface::read_with_extra(name, &ipv4_map, &ipv6_map, &wireless_map) {
                if let Some(pattern) = filter {
                    if !iface.matches_filter(pattern, case_insensitive) {
                        return;
                    }
                }
                iface.write_json(&mut w, opts.verbose, opts.human, &ipv4_map, &ipv6_map);
                count += 1;
            }
        });

        w.end_field_array();
        w.end_object();
        w.finish();
    } else {
        let mut count = 0;
        io::for_each_dir_entry(NET_SYSFS_PATH, |name| {
            if let Some(iface) = NetInterface::read_with_extra(name, &ipv4_map, &ipv6_map, &wireless_map) {
                if let Some(pattern) = filter {
                    if !iface.matches_filter(pattern, case_insensitive) {
                        return;
                    }
                }
                iface.print_text(opts.verbose, opts.human, &ipv4_map, &ipv6_map);
                count += 1;
            }
        });

        if count == 0 {
            if filter.is_some() {
                print::println("net: no matching interfaces");
            } else {
                print::println("net: no network interfaces found");
            }
        }
    }

    0
}

/// Write network interfaces to JSON writer (for snapshot).
#[cfg(feature = "snapshot")]
pub fn write_snapshot(w: &mut StreamingJsonWriter, verbose: bool) {
    if !io::path_exists(NET_SYSFS_PATH) {
        return;
    }

    // Pre-parse all the supplementary data
    let mut wireless_map = WirelessMap::new();
    let mut ipv4_map = Ipv4Map::new();
    let mut ipv6_map = Ipv6Map::new();
    let mut routes = RouteTable::new();

    parse_proc_net_wireless(&mut wireless_map);
    parse_proc_net_if_inet6(&mut ipv6_map);
    parse_proc_net_route(&mut routes);
    parse_proc_net_fib_trie(&mut ipv4_map, &routes);

    w.key("net");
    w.begin_array();
    io::for_each_dir_entry(NET_SYSFS_PATH, |name| {
        if let Some(iface) = NetInterface::read_with_extra(name, &ipv4_map, &ipv6_map, &wireless_map) {
            iface.write_json(w, verbose, false, &ipv4_map, &ipv6_map);
        }
    });
    w.end_array();
}
