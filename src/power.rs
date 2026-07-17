//! Power supply information from /sys/class/power_supply/.
//!
//! Linux exposes batteries, AC adapters, and USB power sources via sysfs.
//! Each power_supply directory contains attributes like type, status,
//! capacity (for batteries), and voltage/current readings.
//!
//! Common types:
//! - Battery: laptop/device batteries with capacity, status, etc.
//! - Mains: AC adapter (online/offline)
//! - USB: USB power delivery sources
//! - UPS: Uninterruptible power supplies

use crate::cli::GlobalOptions;
use crate::fields::power as f;
use crate::filter::{matches_any, opt_str};
use crate::io;
use crate::json::{begin_kv_output_streaming, StreamingJsonWriter};
use crate::print::{self, TextWriter};
use crate::stack::{push_fixed_point, StackString};

const POWER_SUPPLY_PATH: &str = "/sys/class/power_supply";

/// Information about a single power supply.
pub struct PowerSupply {
    /// Supply name (e.g., "BAT0", "AC", "ucsi-source-psy-...")
    pub name: StackString<64>,
    /// Type: Battery, Mains, USB, UPS, etc.
    pub supply_type: Option<StackString<32>>,
    /// Status: Charging, Discharging, Full, Not charging, Unknown
    pub status: Option<StackString<32>>,
    /// Online status (1 = connected, 0 = disconnected) - for Mains/USB
    pub online: Option<u8>,
    /// Battery capacity percentage (0-100)
    pub capacity: Option<u8>,
    /// Voltage now in microvolts
    pub voltage_uv: Option<i64>,
    /// Current now in microamps (positive = charging, negative = discharging)
    pub current_ua: Option<i64>,
    /// Power now in microwatts
    pub power_uw: Option<i64>,
    /// USB type (for USB power supplies): C, PD, PD_PPS, etc.
    pub usb_type: Option<StackString<32>>,
    /// Energy now in microwatt-hours (battery)
    pub energy_now_uwh: Option<i64>,
    /// Energy full in microwatt-hours (battery design capacity)
    pub energy_full_uwh: Option<i64>,
    /// Charge now in microamp-hours (battery)
    pub charge_now_uah: Option<i64>,
    /// Charge full in microamp-hours (battery)
    pub charge_full_uah: Option<i64>,
    /// Battery cycle count
    pub cycle_count: Option<i32>,
    /// Battery technology (Li-ion, Li-poly, NiMH, etc.)
    pub technology: Option<StackString<32>>,
    /// Manufacturer name
    pub manufacturer: Option<StackString<64>>,
    /// Model name
    pub model_name: Option<StackString<64>>,
    /// Maximum current in microamps (USB PD negotiated)
    pub current_max_ua: Option<i64>,
    /// Maximum voltage in microvolts (USB PD negotiated)
    pub voltage_max_uv: Option<i64>,
}

impl PowerSupply {
    /// Read power supply info from sysfs.
    fn read(name: &str) -> Option<Self> {
        let base: StackString<128> = io::join_path(POWER_SUPPLY_PATH, name);

        if !io::path_exists(base.as_str()) {
            return None;
        }

        let type_path: StackString<256> = io::join_path(base.as_str(), "type");
        let status_path: StackString<256> = io::join_path(base.as_str(), "status");
        let online_path: StackString<256> = io::join_path(base.as_str(), "online");
        let capacity_path: StackString<256> = io::join_path(base.as_str(), "capacity");

        let supply_type: Option<StackString<32>> = io::read_file_stack(type_path.as_str());
        let status: Option<StackString<32>> = io::read_file_stack(status_path.as_str());
        let online: Option<u8> = io::read_file_parse(online_path.as_str());
        let capacity: Option<u8> = io::read_file_parse(capacity_path.as_str());

        // Voltage
        let voltage_path: StackString<256> = io::join_path(base.as_str(), "voltage_now");
        let voltage_uv: Option<i64> = io::read_file_parse(voltage_path.as_str());

        // Current
        let current_path: StackString<256> = io::join_path(base.as_str(), "current_now");
        let current_ua: Option<i64> = io::read_file_parse(current_path.as_str());

        // Power
        let power_path: StackString<256> = io::join_path(base.as_str(), "power_now");
        let power_uw: Option<i64> = io::read_file_parse(power_path.as_str());

        // USB type
        let usb_type_path: StackString<256> = io::join_path(base.as_str(), "usb_type");
        let usb_type: Option<StackString<32>> = io::read_file_stack::<64>(usb_type_path.as_str())
            .map(|s| parse_usb_type(s.as_str()));

        // Energy (battery)
        let energy_now_path: StackString<256> = io::join_path(base.as_str(), "energy_now");
        let energy_full_path: StackString<256> = io::join_path(base.as_str(), "energy_full");
        let energy_now_uwh: Option<i64> = io::read_file_parse(energy_now_path.as_str());
        let energy_full_uwh: Option<i64> = io::read_file_parse(energy_full_path.as_str());

        // Charge (battery)
        let charge_now_path: StackString<256> = io::join_path(base.as_str(), "charge_now");
        let charge_full_path: StackString<256> = io::join_path(base.as_str(), "charge_full");
        let charge_now_uah: Option<i64> = io::read_file_parse(charge_now_path.as_str());
        let charge_full_uah: Option<i64> = io::read_file_parse(charge_full_path.as_str());

        // Battery metadata
        let cycle_path: StackString<256> = io::join_path(base.as_str(), "cycle_count");
        let tech_path: StackString<256> = io::join_path(base.as_str(), "technology");
        let mfr_path: StackString<256> = io::join_path(base.as_str(), "manufacturer");
        let model_path: StackString<256> = io::join_path(base.as_str(), "model_name");

        let cycle_count: Option<i32> = io::read_file_parse::<i32>(cycle_path.as_str())
            .filter(|&c| c >= 0);
        let technology: Option<StackString<32>> = io::read_file_stack::<32>(tech_path.as_str())
            .filter(|s| s.as_str() != "Unknown");
        let manufacturer: Option<StackString<64>> = io::read_file_stack(mfr_path.as_str())
            .filter(|s| !s.is_empty());
        let model_name: Option<StackString<64>> = io::read_file_stack(model_path.as_str())
            .filter(|s| !s.is_empty());

        // USB PD limits
        let current_max_path: StackString<256> = io::join_path(base.as_str(), "current_max");
        let voltage_max_path: StackString<256> = io::join_path(base.as_str(), "voltage_max");
        let current_max_ua: Option<i64> = io::read_file_parse(current_max_path.as_str());
        let voltage_max_uv: Option<i64> = io::read_file_parse(voltage_max_path.as_str());

        Some(PowerSupply {
            name: StackString::from_str(name),
            supply_type,
            status,
            online,
            capacity,
            voltage_uv,
            current_ua,
            power_uw,
            usb_type,
            energy_now_uwh,
            energy_full_uwh,
            charge_now_uah,
            charge_full_uah,
            cycle_count,
            technology,
            manufacturer,
            model_name,
            current_max_ua,
            voltage_max_uv,
        })
    }

    /// Check if this supply matches the filter pattern.
    fn matches_filter(&self, pattern: &str, case_insensitive: bool) -> bool {
        let fields = [
            self.name.as_str(),
            opt_str(&self.supply_type),
            opt_str(&self.status),
            opt_str(&self.usb_type),
        ];
        matches_any(&fields, pattern, case_insensitive)
    }

    /// Output as text.
    fn print_text(&self, verbose: bool, human: bool) {
        let mut w = TextWriter::new();

        w.field_str(f::NAME, self.name.as_str());

        if let Some(ref t) = self.supply_type {
            w.field_str(f::TYPE, t.as_str());
        }

        // For batteries, show status and capacity
        let is_battery = self.supply_type.as_ref().map(|s| s.as_str()) == Some("Battery");
        if is_battery {
            if let Some(ref status) = self.status {
                w.field_str(f::STATUS, status.as_str());
            }
            if let Some(cap) = self.capacity {
                let mut cap_str: StackString<16> = StackString::new();
                let mut buf = itoa::Buffer::new();
                cap_str.push_str(buf.format(cap));
                cap_str.push('%');
                w.field_str(f::CAPACITY, cap_str.as_str());
            }
        } else {
            // For Mains/USB, show online status
            if let Some(online) = self.online {
                w.field_str(f::ONLINE, if online == 1 { "yes" } else { "no" });
            }
        }

        // USB type
        if let Some(ref usb_type) = self.usb_type {
            w.field_str(f::USB_TYPE, usb_type.as_str());
        }

        if verbose {
            // Energy (batteries)
            if let (Some(now), Some(full)) = (self.energy_now_uwh, self.energy_full_uwh) {
                if human {
                    let mut s: StackString<32> = StackString::new();
                    format_energy_pair(&mut s, now, full);
                    w.field_str(f::ENERGY, s.as_str());
                } else {
                    // Format as now/full in Wh with 1 decimal
                    let mut s: StackString<32> = StackString::new();
                    format_uwh_pair(&mut s, now, full);
                    w.field_str(f::ENERGY_WH, s.as_str());
                }
            } else if let (Some(now), Some(full)) = (self.charge_now_uah, self.charge_full_uah) {
                if human {
                    let mut s: StackString<32> = StackString::new();
                    format_charge_pair(&mut s, now, full);
                    w.field_str(f::CHARGE, s.as_str());
                } else {
                    let mut s: StackString<32> = StackString::new();
                    format_uah_pair(&mut s, now, full);
                    w.field_str(f::CHARGE_MAH, s.as_str());
                }
            }

            // Voltage
            if let Some(v) = self.voltage_uv {
                if human {
                    let mut s: StackString<16> = StackString::new();
                    format_uv_human(&mut s, v);
                    w.field_str(f::VOLTAGE, s.as_str());
                } else {
                    let mut s: StackString<16> = StackString::new();
                    format_uv_decimal(&mut s, v);
                    w.field_str(f::VOLTAGE_V, s.as_str());
                }
            }

            // Current
            if let Some(c) = self.current_ua {
                if human {
                    let mut s: StackString<16> = StackString::new();
                    format_ua_human(&mut s, c);
                    w.field_str(f::CURRENT, s.as_str());
                } else {
                    let mut s: StackString<16> = StackString::new();
                    format_ua_decimal(&mut s, c);
                    w.field_str(f::CURRENT_A, s.as_str());
                }
            }

            // Power
            if let Some(p) = self.power_uw {
                if human {
                    let mut s: StackString<16> = StackString::new();
                    format_uw_human(&mut s, p);
                    w.field_str(f::POWER, s.as_str());
                } else {
                    let mut s: StackString<16> = StackString::new();
                    format_uw_decimal(&mut s, p);
                    w.field_str(f::POWER_W, s.as_str());
                }
            }

            // USB PD limits
            if let Some(v) = self.voltage_max_uv {
                if human {
                    let mut s: StackString<16> = StackString::new();
                    format_uv_human(&mut s, v);
                    w.field_str(f::VOLTAGE_MAX, s.as_str());
                } else {
                    let mut s: StackString<16> = StackString::new();
                    format_uv_decimal(&mut s, v);
                    w.field_str(f::VOLTAGE_MAX_V, s.as_str());
                }
            }
            if let Some(c) = self.current_max_ua {
                if human {
                    let mut s: StackString<16> = StackString::new();
                    format_ua_human(&mut s, c);
                    w.field_str(f::CURRENT_MAX, s.as_str());
                } else {
                    let mut s: StackString<16> = StackString::new();
                    format_ua_decimal(&mut s, c);
                    w.field_str(f::CURRENT_MAX_A, s.as_str());
                }
            }

            // Battery metadata
            if let Some(cycles) = self.cycle_count {
                w.field_u64(f::CYCLES, cycles as u64);
            }
            if let Some(ref tech) = self.technology {
                w.field_str(f::TECHNOLOGY, tech.as_str());
            }
            if let Some(ref model) = self.model_name {
                w.field_quoted(f::MODEL, model.as_str());
            }
            if let Some(ref mfr) = self.manufacturer {
                w.field_quoted(f::MANUFACTURER, mfr.as_str());
            }
        }

        w.finish();
    }

    /// Write as JSON object.
    fn write_json(&self, w: &mut StreamingJsonWriter, verbose: bool) {
        w.array_object_begin();

        w.field_str(f::NAME, self.name.as_str());

        if let Some(ref t) = self.supply_type {
            w.field_str(f::TYPE, t.as_str());
        }

        if let Some(ref status) = self.status {
            w.field_str(f::STATUS, status.as_str());
        }

        if let Some(online) = self.online {
            w.field_bool(f::ONLINE, online == 1);
        }

        if let Some(cap) = self.capacity {
            w.field_u64(f::CAPACITY_PERCENT, cap as u64);
        }

        if let Some(ref usb_type) = self.usb_type {
            w.field_str(f::USB_TYPE, usb_type.as_str());
        }

        if verbose {
            if let Some(v) = self.voltage_uv {
                w.field_i64(f::VOLTAGE_UV, v);
            }
            if let Some(c) = self.current_ua {
                w.field_i64(f::CURRENT_UA, c);
            }
            if let Some(p) = self.power_uw {
                w.field_i64(f::POWER_UW, p);
            }
            if let Some(e) = self.energy_now_uwh {
                w.field_i64(f::ENERGY_NOW_UWH, e);
            }
            if let Some(e) = self.energy_full_uwh {
                w.field_i64(f::ENERGY_FULL_UWH, e);
            }
            if let Some(c) = self.charge_now_uah {
                w.field_i64(f::CHARGE_NOW_UAH, c);
            }
            if let Some(c) = self.charge_full_uah {
                w.field_i64(f::CHARGE_FULL_UAH, c);
            }
            if let Some(v) = self.voltage_max_uv {
                w.field_i64(f::VOLTAGE_MAX_UV, v);
            }
            if let Some(c) = self.current_max_ua {
                w.field_i64(f::CURRENT_MAX_UA, c);
            }
            if let Some(cycles) = self.cycle_count {
                w.field_i64(f::CYCLE_COUNT, cycles as i64);
            }
            if let Some(ref tech) = self.technology {
                w.field_str(f::TECHNOLOGY, tech.as_str());
            }
            if let Some(ref model) = self.model_name {
                w.field_str(f::MODEL_NAME, model.as_str());
            }
            if let Some(ref mfr) = self.manufacturer {
                w.field_str(f::MANUFACTURER, mfr.as_str());
            }
        }

        w.array_object_end();
    }
}

/// Parse USB type string - extract the active type marked with [brackets].
fn parse_usb_type(s: &str) -> StackString<32> {
    // Format: "C [PD] PD_PPS" - extract what's in brackets
    if let Some(start) = s.find('[') {
        if let Some(end) = s.find(']') {
            if start < end {
                return StackString::from_str(&s[start + 1..end]);
            }
        }
    }
    // No brackets - return as-is (trimmed)
    StackString::from_str(s.trim())
}

/// Format microvolts as human-readable (e.g., "12.5V").
fn format_uv_human(s: &mut StackString<16>, uv: i64) {
    push_fixed_point(s, uv / 100_000, 1);
    s.push('V');
}

/// Format microvolts as decimal volts (e.g., "12.50").
fn format_uv_decimal(s: &mut StackString<16>, uv: i64) {
    push_fixed_point(s, uv / 10_000, 2);
}

/// Format microamps as human-readable (e.g., "1.5A" or "500mA").
fn format_ua_human(s: &mut StackString<16>, ua: i64) {
    if ua.unsigned_abs() >= 1_000_000 {
        push_fixed_point(s, ua / 100_000, 1);
        s.push('A');
    } else {
        push_fixed_point(s, ua / 1000, 0);
        s.push_str("mA");
    }
}

/// Format microamps as decimal amps (e.g., "1.500").
fn format_ua_decimal(s: &mut StackString<16>, ua: i64) {
    push_fixed_point(s, ua / 1000, 3);
}

/// Format microwatts as human-readable (e.g., "18.8W").
fn format_uw_human(s: &mut StackString<16>, uw: i64) {
    push_fixed_point(s, uw / 100_000, 1);
    s.push('W');
}

/// Format microwatts as decimal watts (e.g., "18.75").
fn format_uw_decimal(s: &mut StackString<16>, uw: i64) {
    push_fixed_point(s, uw / 10_000, 2);
}

/// Format energy pair as human-readable (e.g., "45.0Wh/50.0Wh").
fn format_energy_pair(s: &mut StackString<32>, now_uwh: i64, full_uwh: i64) {
    push_fixed_point(s, now_uwh / 100_000, 1);
    s.push_str("Wh/");
    push_fixed_point(s, full_uwh / 100_000, 1);
    s.push_str("Wh");
}

/// Format energy pair as decimal Wh (e.g., "45.0/50.0").
fn format_uwh_pair(s: &mut StackString<32>, now_uwh: i64, full_uwh: i64) {
    push_fixed_point(s, now_uwh / 100_000, 1);
    s.push('/');
    push_fixed_point(s, full_uwh / 100_000, 1);
}

/// Format charge pair as human-readable (e.g., "4500mAh/5000mAh").
fn format_charge_pair(s: &mut StackString<32>, now_uah: i64, full_uah: i64) {
    let now_mah = now_uah / 1000;
    let full_mah = full_uah / 1000;
    let mut buf = itoa::Buffer::new();
    s.push_str(buf.format(now_mah));
    s.push_str("mAh/");
    s.push_str(buf.format(full_mah));
    s.push_str("mAh");
}

/// Format charge pair as decimal mAh (e.g., "4500/5000").
fn format_uah_pair(s: &mut StackString<32>, now_uah: i64, full_uah: i64) {
    let now_mah = now_uah / 1000;
    let full_mah = full_uah / 1000;
    let mut buf = itoa::Buffer::new();
    s.push_str(buf.format(now_mah));
    s.push('/');
    s.push_str(buf.format(full_mah));
}

/// Entry point for `kv power` subcommand.
pub fn run(opts: &GlobalOptions) -> i32 {
    if !io::path_exists(POWER_SUPPLY_PATH) {
        if opts.json {
            let mut w = begin_kv_output_streaming(opts.pretty, "power");
            w.field_array("data");
            w.end_field_array();
            w.end_object();
            w.finish();
        } else {
            print::println("power: no power supplies found");
        }
        return 0;
    }

    let filter = opts.filter.as_ref().map(|s| s.as_str());
    let case_insensitive = opts.filter_case_insensitive;

    if opts.json {
        let mut w = begin_kv_output_streaming(opts.pretty, "power");
        w.field_array("data");

        let mut count = 0;
        io::for_each_dir_entry(POWER_SUPPLY_PATH, |name| {
            if let Some(supply) = PowerSupply::read(name) {
                if let Some(pattern) = filter {
                    if !supply.matches_filter(pattern, case_insensitive) {
                        return;
                    }
                }
                supply.write_json(&mut w, opts.verbose);
                count += 1;
            }
        });

        w.end_field_array();
        w.end_object();
        w.finish();
    } else {
        let mut count = 0;
        io::for_each_dir_entry(POWER_SUPPLY_PATH, |name| {
            if let Some(supply) = PowerSupply::read(name) {
                if let Some(pattern) = filter {
                    if !supply.matches_filter(pattern, case_insensitive) {
                        return;
                    }
                }
                supply.print_text(opts.verbose, opts.human);
                count += 1;
            }
        });

        if count == 0 {
            if filter.is_some() {
                print::println("power: no matching power supplies");
            } else {
                print::println("power: no power supplies found");
            }
        }
    }

    0
}

/// Write power supplies to JSON writer (for snapshot).
#[cfg(feature = "snapshot")]
pub fn write_snapshot(w: &mut StreamingJsonWriter, verbose: bool) {
    if !io::path_exists(POWER_SUPPLY_PATH) {
        return;
    }

    let mut has_any = false;
    io::for_each_dir_entry(POWER_SUPPLY_PATH, |_| {
        has_any = true;
    });

    if !has_any {
        return;
    }

    w.key("power");
    w.begin_array();
    io::for_each_dir_entry(POWER_SUPPLY_PATH, |name| {
        if let Some(supply) = PowerSupply::read(name) {
            supply.write_json(w, verbose);
        }
    });
    w.end_array();
}

#[cfg(test)]
mod format_tests {
    use super::*;

    fn fmt16(f: impl Fn(&mut StackString<16>)) -> std::string::String {
        let mut s: StackString<16> = StackString::new();
        f(&mut s);
        s.as_str().to_string()
    }

    #[test]
    fn uv_decimal_positive() {
        assert_eq!(fmt16(|s| format_uv_decimal(s, 12_500_000)), "12.50");
    }

    #[test]
    fn uv_decimal_keeps_sign_when_below_one() {
        // A discharging reading of -0.8 must not print as 0.80
        assert_eq!(fmt16(|s| format_uv_decimal(s, -800_000)), "-0.80");
    }

    #[test]
    fn ua_decimal_keeps_sign_when_below_one() {
        assert_eq!(fmt16(|s| format_ua_decimal(s, -800_000)), "-0.800");
    }

    #[test]
    fn uw_decimal_keeps_sign_when_below_one() {
        assert_eq!(fmt16(|s| format_uw_decimal(s, -500_000)), "-0.50");
    }

    #[test]
    fn ua_human_keeps_sign() {
        assert_eq!(fmt16(|s| format_ua_human(s, -800_000)), "-800mA");
        assert_eq!(fmt16(|s| format_ua_human(s, 1_500_000)), "1.5A");
    }
}
