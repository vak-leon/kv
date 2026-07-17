//! Canonical field names for consistent output across text and JSON formats.
//!
//! All field names are defined once here in lowercase_with_underscores format.
//! TextWriter handles uppercase conversion for text output automatically.
//!
//! This ensures the field name `state` produces:
//! - Text: `STATE=up`
//! - JSON: `"state": "up"`

/// Network interface fields (kv net)
pub mod net {
    pub const NAME: &str = "name";
    pub const MAC: &str = "mac";
    pub const MTU: &str = "mtu";
    pub const STATE: &str = "state";
    pub const SPEED: &str = "speed";
    pub const DUPLEX: &str = "duplex";
    pub const CARRIER: &str = "carrier";
    pub const IP: &str = "ip";
    pub const IPV4: &str = "ipv4";
    pub const IPV6: &str = "ipv6";
    pub const SIGNAL: &str = "signal";
    pub const LINK: &str = "link";
    pub const NOISE: &str = "noise";
    pub const RX_BYTES: &str = "rx_bytes";
    pub const TX_BYTES: &str = "tx_bytes";
    pub const RX_PACKETS: &str = "rx_packets";
    pub const TX_PACKETS: &str = "tx_packets";
    pub const RX_ERRORS: &str = "rx_errors";
    pub const TX_ERRORS: &str = "tx_errors";
    pub const RX_DROPPED: &str = "rx_dropped";
    pub const TX_DROPPED: &str = "tx_dropped";
}

/// Memory fields (kv mem)
pub mod mem {
    // With _kb suffix (raw mode)
    pub const MEM_TOTAL_KB: &str = "mem_total_kb";
    pub const MEM_FREE_KB: &str = "mem_free_kb";
    pub const MEM_AVAILABLE_KB: &str = "mem_available_kb";
    pub const SWAP_TOTAL_KB: &str = "swap_total_kb";
    pub const SWAP_FREE_KB: &str = "swap_free_kb";
    pub const BUFFERS_KB: &str = "buffers_kb";
    pub const CACHED_KB: &str = "cached_kb";
    pub const SWAP_CACHED_KB: &str = "swap_cached_kb";
    pub const SHMEM_KB: &str = "shmem_kb";
    pub const SRECLAIMABLE_KB: &str = "sreclaimable_kb";
    pub const SUNRECLAIM_KB: &str = "sunreclaim_kb";
    pub const DIRTY_KB: &str = "dirty_kb";
    pub const WRITEBACK_KB: &str = "writeback_kb";

    // Without _kb suffix (human mode)
    pub const MEM_TOTAL: &str = "mem_total";
    pub const MEM_FREE: &str = "mem_free";
    pub const MEM_AVAILABLE: &str = "mem_available";
    pub const SWAP_TOTAL: &str = "swap_total";
    pub const SWAP_FREE: &str = "swap_free";
    pub const BUFFERS: &str = "buffers";
    pub const CACHED: &str = "cached";
    pub const SWAP_CACHED: &str = "swap_cached";
    pub const SHMEM: &str = "shmem";
    pub const SRECLAIMABLE: &str = "sreclaimable";
    pub const SUNRECLAIM: &str = "sunreclaim";
    pub const DIRTY: &str = "dirty";
    pub const WRITEBACK: &str = "writeback";
}

/// PCI device fields (kv pci)
pub mod pci {
    pub const BDF: &str = "bdf";
    pub const VENDOR_ID: &str = "vendor_id";
    pub const DEVICE_ID: &str = "device_id";
    pub const CLASS: &str = "class";
    pub const DRIVER: &str = "driver";
    pub const SUBSYS_VENDOR: &str = "subsys_vendor";
    pub const SUBSYS_DEVICE: &str = "subsys_device";
    pub const REVISION: &str = "revision";
    pub const NUMA_NODE: &str = "numa_node";
    pub const IOMMU_GROUP: &str = "iommu_group";
    pub const ENABLED: &str = "enabled";
    pub const POWER_STATE: &str = "power_state";
    pub const IS_BRIDGE: &str = "is_bridge";
}

/// Block device fields (kv block)
pub mod block {
    pub const NAME: &str = "name";
    pub const TYPE: &str = "type";
    pub const MAJOR: &str = "major";
    pub const MINOR: &str = "minor";
    pub const MAJMIN: &str = "majmin";
    pub const SIZE: &str = "size";
    pub const SIZE_SECTORS: &str = "size_sectors";
    pub const PARENT: &str = "parent";
    pub const MOUNTPOINT: &str = "mountpoint";
    pub const SECTOR_SIZE: &str = "sector_size";
    pub const REMOVABLE: &str = "removable";
    pub const RO: &str = "ro";
    pub const MODEL: &str = "model";
    pub const ROTATIONAL: &str = "rotational";
    pub const SCHEDULER: &str = "scheduler";
}

/// CPU fields (kv cpu)
pub mod cpu {
    pub const LOGICAL_CPUS: &str = "logical_cpus";
    pub const MODEL_NAME: &str = "model_name";
    pub const VENDOR_ID: &str = "vendor_id";
    pub const SOCKETS: &str = "sockets";
    pub const CORES_PER_SOCKET: &str = "cores_per_socket";
    pub const ISA: &str = "isa";
    pub const MMU: &str = "mmu";
    pub const CPU_FAMILY: &str = "cpu_family";
    pub const MODEL: &str = "model";
    pub const STEPPING: &str = "stepping";
    pub const CPU_MHZ: &str = "cpu_mhz";
    pub const CACHE_SIZE: &str = "cache_size";
    pub const ARCHITECTURE: &str = "architecture";
    pub const FLAGS: &str = "flags";
}

/// Thermal fields (kv thermal)
pub mod thermal {
    pub const SENSOR: &str = "sensor";
    pub const LABEL: &str = "label";
    pub const TEMP: &str = "temp";
    pub const TEMP_MILLICELSIUS: &str = "temp_millicelsius";
    pub const CRIT: &str = "crit";
    pub const TEMP_CRIT_MILLICELSIUS: &str = "temp_crit_millicelsius";
    pub const TRIPS: &str = "trips";
    pub const TRIP_POINTS: &str = "trip_points";
    pub const POLICY: &str = "policy";
    pub const SOURCE: &str = "source";
    pub const NAME: &str = "name";
    pub const COOLING: &str = "cooling";
    pub const TYPE: &str = "type";
    pub const CUR_STATE: &str = "cur_state";
    pub const MAX_STATE: &str = "max_state";
    pub const INDEX: &str = "index";
    pub const STATE: &str = "state";
}

/// Power supply fields (kv power)
pub mod power {
    pub const NAME: &str = "name";
    pub const TYPE: &str = "type";
    pub const STATUS: &str = "status";
    pub const ONLINE: &str = "online";
    pub const CAPACITY: &str = "capacity";
    pub const CAPACITY_PERCENT: &str = "capacity_percent";
    pub const USB_TYPE: &str = "usb_type";
    pub const VOLTAGE_UV: &str = "voltage_uv";
    pub const VOLTAGE_V: &str = "voltage_v";
    pub const VOLTAGE: &str = "voltage";
    pub const CURRENT_UA: &str = "current_ua";
    pub const CURRENT_A: &str = "current_a";
    pub const CURRENT: &str = "current";
    pub const POWER_UW: &str = "power_uw";
    pub const POWER_W: &str = "power_w";
    pub const POWER: &str = "power";
    pub const ENERGY: &str = "energy";
    pub const ENERGY_WH: &str = "energy_wh";
    pub const ENERGY_NOW_UWH: &str = "energy_now_uwh";
    pub const ENERGY_FULL_UWH: &str = "energy_full_uwh";
    pub const CHARGE: &str = "charge";
    pub const CHARGE_MAH: &str = "charge_mah";
    pub const CHARGE_NOW_UAH: &str = "charge_now_uah";
    pub const CHARGE_FULL_UAH: &str = "charge_full_uah";
    pub const VOLTAGE_MAX_UV: &str = "voltage_max_uv";
    pub const VOLTAGE_MAX_V: &str = "voltage_max_v";
    pub const VOLTAGE_MAX: &str = "voltage_max";
    pub const CURRENT_MAX_UA: &str = "current_max_ua";
    pub const CURRENT_MAX_A: &str = "current_max_a";
    pub const CURRENT_MAX: &str = "current_max";
    pub const CYCLES: &str = "cycles";
    pub const CYCLE_COUNT: &str = "cycle_count";
    pub const TECHNOLOGY: &str = "technology";
    pub const MODEL_NAME: &str = "model_name";
    pub const MODEL: &str = "model";
    pub const MANUFACTURER: &str = "manufacturer";
}

/// USB device fields (kv usb)
pub mod usb {
    pub const NAME: &str = "name";
    pub const VENDOR_ID: &str = "vendor_id";
    pub const PRODUCT_ID: &str = "product_id";
    pub const MANUFACTURER: &str = "manufacturer";
    pub const PRODUCT: &str = "product";
    pub const SPEED_MBPS: &str = "speed_mbps";
    pub const DEVICE_CLASS: &str = "device_class";
    pub const CLASS: &str = "class";
    pub const BUSNUM: &str = "busnum";
    pub const BUS: &str = "bus";
    pub const DEVNUM: &str = "devnum";
    pub const DEV: &str = "dev";
    pub const SERIAL: &str = "serial";
    pub const USB_VERSION: &str = "usb_version";
    pub const NUM_CONFIGURATIONS: &str = "num_configurations";
    pub const CONFIGURATION: &str = "configuration";
    pub const MAX_POWER_MA: &str = "max_power_ma";
    pub const DRIVER: &str = "driver";
}

/// Device tree fields (kv dt)
pub mod dt {
    pub const PATH: &str = "path";
    pub const NAME: &str = "name";
    pub const COMPATIBLE: &str = "compatible";
    pub const STATUS: &str = "status";
    pub const MODEL: &str = "model";
    pub const NODE_COUNT: &str = "node_count";
    pub const PROPERTIES: &str = "properties";
    pub const REG: &str = "reg";
}

/// Mount point fields (kv mounts)
pub mod mounts {
    pub const SOURCE: &str = "source";
    pub const TARGET: &str = "target";
    pub const FSTYPE: &str = "fstype";
    pub const OPTIONS: &str = "options";
    pub const DUMP_FREQ: &str = "dump_freq";
    pub const PASS_NUM: &str = "pass_num";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_constants_are_lowercase() {
        // Spot check a few fields to ensure they're lowercase
        assert_eq!(net::STATE, "state");
        assert_eq!(net::MAC, "mac");
        assert_eq!(mem::MEM_TOTAL_KB, "mem_total_kb");
        assert_eq!(pci::VENDOR_ID, "vendor_id");
    }
}
