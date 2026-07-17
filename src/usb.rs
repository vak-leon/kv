//! USB device information from /sys/bus/usb/devices.
//!
//! The USB sysfs hierarchy is... interesting. Unlike PCI where you have
//! nice clean BDF addresses, USB has a tree structure where devices are
//! named things like "1-1.4.2" (bus 1, port 1, hub port 4, hub port 2).
//!
//! We enumerate devices from /sys/bus/usb/devices and skip the "usb*"
//! root hub entries since those aren't real devices users care about.
//!
//! Note: USB device strings (manufacturer, product) might require special
//! permissions to read on some systems. We gracefully handle missing strings.

use crate::cli::GlobalOptions;
use crate::fields::usb as f;
use crate::filter::{matches_any, opt_str};
use crate::io;
use crate::json::{begin_kv_output_streaming, StreamingJsonWriter};
use crate::print::{self, TextWriter};
use crate::stack::StackString;

const USB_SYSFS_PATH: &str = "/sys/bus/usb/devices";

/// Information about a USB device.
pub struct UsbDevice {
    /// Device name in USB topology (e.g., "1-1.4")
    pub name: StackString<16>,
    /// Vendor ID
    pub vendor_id: u16,
    /// Product ID
    pub product_id: u16,
    /// Device class
    pub device_class: u8,
    /// Bus number
    pub busnum: u8,
    /// Device number
    pub devnum: u8,
    /// USB speed (in Mbps: 1.5, 12, 480, 5000, 10000, 20000)
    pub speed_mbps: Option<u32>,
    /// Manufacturer string
    pub manufacturer: Option<StackString<64>>,
    /// Product string
    pub product: Option<StackString<64>>,
    /// Serial number
    pub serial: Option<StackString<64>>,
    /// USB version (e.g., "2.00", "3.10")
    pub usb_version: Option<StackString<16>>,
    /// Number of configurations
    pub num_configurations: Option<u8>,
    /// Current configuration value
    pub configuration: Option<u8>,
    /// Maximum power consumption in mA
    pub max_power_ma: Option<u32>,
    /// Bound driver
    pub driver: Option<StackString<32>>,
}

impl UsbDevice {
    /// Read a USB device from sysfs.
    pub fn read(name: &str) -> Option<Self> {
        // Skip root hubs (usb1, usb2, etc.) - they're not real devices
        if name.starts_with("usb") {
            return None;
        }

        // Skip interface directories (contain ':')
        if name.contains(':') {
            return None;
        }

        let base: StackString<64> = io::join_path(USB_SYSFS_PATH, name);

        // Must have vendor and product IDs
        let vendor_path: StackString<128> = io::join_path(base.as_str(), "idVendor");
        let product_path: StackString<128> = io::join_path(base.as_str(), "idProduct");
        let vendor_id: u16 = io::read_file_hex(vendor_path.as_str())?;
        let product_id: u16 = io::read_file_hex(product_path.as_str())?;

        let class_path: StackString<128> = io::join_path(base.as_str(), "bDeviceClass");
        let busnum_path: StackString<128> = io::join_path(base.as_str(), "busnum");
        let devnum_path: StackString<128> = io::join_path(base.as_str(), "devnum");
        let device_class: u8 = io::read_file_hex(class_path.as_str()).unwrap_or(0);
        let busnum: u8 = io::read_file_parse(busnum_path.as_str()).unwrap_or(0);
        let devnum: u8 = io::read_file_parse(devnum_path.as_str()).unwrap_or(0);

        // Speed is reported as a string like "480" or "5000"
        let speed_path: StackString<128> = io::join_path(base.as_str(), "speed");
        let speed_mbps: Option<u32> = io::read_file_parse(speed_path.as_str());

        // These may require elevated permissions
        let mfr_path: StackString<128> = io::join_path(base.as_str(), "manufacturer");
        let prod_path: StackString<128> = io::join_path(base.as_str(), "product");
        let serial_path: StackString<128> = io::join_path(base.as_str(), "serial");
        let manufacturer: Option<StackString<64>> = io::read_file_stack(mfr_path.as_str());
        let product: Option<StackString<64>> = io::read_file_stack(prod_path.as_str());
        let serial: Option<StackString<64>> = io::read_file_stack(serial_path.as_str());

        let version_path: StackString<128> = io::join_path(base.as_str(), "version");
        let usb_version: Option<StackString<16>> = io::read_file_stack(version_path.as_str());

        let numconf_path: StackString<128> = io::join_path(base.as_str(), "bNumConfigurations");
        let confval_path: StackString<128> = io::join_path(base.as_str(), "bConfigurationValue");
        let num_configurations: Option<u8> = io::read_file_parse(numconf_path.as_str());
        let configuration: Option<u8> = io::read_file_parse(confval_path.as_str());

        // Max power is in mA but sometimes reported as "500mA" string
        let power_path: StackString<128> = io::join_path(base.as_str(), "bMaxPower");
        let max_power_ma: Option<u32> = io::read_file_stack::<16>(power_path.as_str())
            .and_then(|s| {
                let trimmed = s.as_str().strip_suffix("mA").unwrap_or(s.as_str());
                trimmed.parse().ok()
            });

        let driver_path: StackString<128> = io::join_path(base.as_str(), "driver");
        let driver: Option<StackString<32>> = io::read_symlink_name(driver_path.as_str());

        Some(UsbDevice {
            name: StackString::from_str(name),
            vendor_id,
            product_id,
            device_class,
            busnum,
            devnum,
            speed_mbps,
            manufacturer,
            product,
            serial,
            usb_version,
            num_configurations,
            configuration,
            max_power_ma,
            driver,
        })
    }

    /// Check if this device matches the filter pattern.
    fn matches_filter(&self, pattern: &str, case_insensitive: bool) -> bool {
        let vendor_hex = io::format_hex_u16(self.vendor_id);
        let product_hex = io::format_hex_u16(self.product_id);
        let fields = [
            self.name.as_str(),
            opt_str(&self.manufacturer),
            opt_str(&self.product),
            vendor_hex.as_str(),
            product_hex.as_str(),
        ];
        matches_any(&fields, pattern, case_insensitive)
    }

    /// Output as text.
    fn print_text(&self, verbose: bool) {
        let mut w = TextWriter::new();

        w.field_str(f::NAME, self.name.as_str());
        w.field_str(f::VENDOR_ID, io::format_hex_u16(self.vendor_id).as_str());
        w.field_str(f::PRODUCT_ID, io::format_hex_u16(self.product_id).as_str());

        if let Some(ref mfr) = self.manufacturer {
            w.field_quoted(f::MANUFACTURER, mfr.as_str());
        }
        if let Some(ref prod) = self.product {
            w.field_quoted(f::PRODUCT, prod.as_str());
        }
        if let Some(speed) = self.speed_mbps {
            w.field_u64(f::SPEED_MBPS, speed as u64);
        }

        if verbose {
            w.field_str(f::DEVICE_CLASS, io::format_hex_u8(self.device_class).as_str());
            w.field_u64(f::BUSNUM, self.busnum as u64);
            w.field_u64(f::DEVNUM, self.devnum as u64);
            if let Some(ref serial) = self.serial {
                w.field_quoted(f::SERIAL, serial.as_str());
            }
            if let Some(ref version) = self.usb_version {
                w.field_str(f::USB_VERSION, version.as_str());
            }
            if let Some(power) = self.max_power_ma {
                w.field_u64(f::MAX_POWER_MA, power as u64);
            }
            if let Some(ref driver) = self.driver {
                w.field_str(f::DRIVER, driver.as_str());
            }
        }

        w.finish();
    }

    /// Write as JSON object.
    fn write_json(&self, w: &mut StreamingJsonWriter, verbose: bool) {
        w.array_object_begin();

        w.field_str(f::NAME, self.name.as_str());
        w.field_str(f::VENDOR_ID, io::format_hex_u16(self.vendor_id).as_str());
        w.field_str(f::PRODUCT_ID, io::format_hex_u16(self.product_id).as_str());
        w.field_str_opt(f::MANUFACTURER, self.manufacturer.as_ref().map(|s| s.as_str()));
        w.field_str_opt(f::PRODUCT, self.product.as_ref().map(|s| s.as_str()));
        w.field_u64_opt(f::SPEED_MBPS, self.speed_mbps.map(|v| v as u64));

        if verbose {
            w.field_str(f::DEVICE_CLASS, io::format_hex_u8(self.device_class).as_str());
            w.field_u64(f::BUSNUM, self.busnum as u64);
            w.field_u64(f::DEVNUM, self.devnum as u64);
            w.field_str_opt(f::SERIAL, self.serial.as_ref().map(|s| s.as_str()));
            w.field_str_opt(f::USB_VERSION, self.usb_version.as_ref().map(|s| s.as_str()));
            w.field_u64_opt(f::NUM_CONFIGURATIONS, self.num_configurations.map(|v| v as u64));
            w.field_u64_opt(f::CONFIGURATION, self.configuration.map(|v| v as u64));
            w.field_u64_opt(f::MAX_POWER_MA, self.max_power_ma.map(|v| v as u64));
            w.field_str_opt(f::DRIVER, self.driver.as_ref().map(|s| s.as_str()));
        }

        w.array_object_end();
    }
}

/// Entry point for `kv usb` subcommand.
pub fn run(opts: &GlobalOptions) -> i32 {
    if !io::path_exists(USB_SYSFS_PATH) {
        if opts.json {
            let mut w = begin_kv_output_streaming(opts.pretty, "usb");
            w.field_array("data");
            w.end_field_array();
            w.end_object();
            w.finish();
        } else {
            print::println("usb: no USB bus found");
        }
        return 0;
    }

    let filter = opts.filter.as_ref().map(|s| s.as_str());
    let case_insensitive = opts.filter_case_insensitive;

    if opts.json {
        let mut w = begin_kv_output_streaming(opts.pretty, "usb");
        w.field_array("data");

        let mut count = 0;
        io::for_each_dir_entry(USB_SYSFS_PATH, |name| {
            if let Some(dev) = UsbDevice::read(name) {
                if let Some(pattern) = filter {
                    if !dev.matches_filter(pattern, case_insensitive) {
                        return;
                    }
                }
                dev.write_json(&mut w, opts.verbose);
                count += 1;
            }
        });

        w.end_field_array();
        w.end_object();
        w.finish();
    } else {
        let mut count = 0;
        io::for_each_dir_entry(USB_SYSFS_PATH, |name| {
            if let Some(dev) = UsbDevice::read(name) {
                if let Some(pattern) = filter {
                    if !dev.matches_filter(pattern, case_insensitive) {
                        return;
                    }
                }
                dev.print_text(opts.verbose);
                count += 1;
            }
        });

        if count == 0 {
            if filter.is_some() {
                print::println("usb: no matching devices");
            } else {
                print::println("usb: no USB devices found");
            }
        }
    }

    0
}

/// Write USB devices to JSON writer (for snapshot).
#[cfg(feature = "snapshot")]
pub fn write_snapshot(w: &mut StreamingJsonWriter, verbose: bool) {
    if !io::path_exists(USB_SYSFS_PATH) {
        return;
    }

    w.key("usb");
    w.begin_array();
    io::for_each_dir_entry(USB_SYSFS_PATH, |name| {
        if let Some(dev) = UsbDevice::read(name) {
            dev.write_json(w, verbose);
        }
    });
    w.end_array();
}
