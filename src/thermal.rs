//! Thermal sensor information from /sys/class/thermal/ and /sys/class/hwmon/.
//!
//! Linux exposes thermal data via two subsystems:
//!
//! 1. thermal_zone - High-level thermal management with named zones
//!    (like "cpu-thermal", "gpu-thermal"). Common on ARM/embedded.
//!
//! 2. hwmon - Hardware monitoring with raw sensor data. Common on x86
//!    (coretemp, k10temp, it87, etc.) and provides more detail.
//!
//! We also expose cooling devices (fans, CPU frequency scaling, throttle
//! alerts) and trip points (temperature thresholds that trigger actions).
//!
//! Temperature is reported in millidegrees Celsius - divide by 1000
//! for the human-readable value. We keep it in millidegrees for precision.

use crate::cli::GlobalOptions;
use crate::fields::thermal as f;
use crate::filter::{matches_any, opt_str};
use crate::io;
use crate::json::{begin_kv_output_streaming, StreamingJsonWriter};
use crate::print::{self, TextWriter};
use crate::stack::{push_fixed_point, StackString};

const THERMAL_PATH: &str = "/sys/class/thermal";
const HWMON_PATH: &str = "/sys/class/hwmon";

/// Source of thermal data.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ThermalSource {
    /// From /sys/class/thermal/thermal_zone*
    ThermalZone,
    /// From /sys/class/hwmon/hwmon*/temp*_input
    Hwmon,
}

impl ThermalSource {
    fn as_str(&self) -> &'static str {
        match self {
            ThermalSource::ThermalZone => "thermal",
            ThermalSource::Hwmon => "hwmon",
        }
    }
}

/// Information about a single thermal sensor.
pub struct ThermalZone {
    /// Zone/sensor name (e.g., "thermal_zone0", "hwmon0")
    pub name: StackString<32>,
    /// Type/label (e.g., "cpu-thermal", "coretemp")
    pub zone_type: Option<StackString<64>>,
    /// Sensor label (for hwmon, e.g., "Core 0", "Package id 0")
    pub label: Option<StackString<64>>,
    /// Current temperature in millidegrees Celsius
    pub temp_millicelsius: Option<i64>,
    /// Policy in use (e.g., "step_wise") - thermal zones only
    pub policy: Option<StackString<32>>,
    /// Critical temperature threshold in millidegrees
    pub temp_crit: Option<i64>,
    /// Source of this reading
    pub source: ThermalSource,
}

impl ThermalZone {
    /// Read thermal zone info from /sys/class/thermal.
    fn read_thermal_zone(name: &str) -> Option<Self> {
        if !name.starts_with("thermal_zone") {
            return None;
        }

        let base: StackString<128> = io::join_path(THERMAL_PATH, name);

        let type_path: StackString<128> = io::join_path(base.as_str(), "type");
        let temp_path: StackString<128> = io::join_path(base.as_str(), "temp");
        let policy_path: StackString<128> = io::join_path(base.as_str(), "policy");

        let zone_type: Option<StackString<64>> = io::read_file_stack(type_path.as_str());
        let temp_millicelsius: Option<i64> = io::read_file_parse(temp_path.as_str());
        let policy: Option<StackString<32>> = io::read_file_stack(policy_path.as_str());

        // Find critical temperature from trip points
        let temp_crit = find_critical_trip_point(base.as_str());

        Some(ThermalZone {
            name: StackString::from_str(name),
            zone_type,
            label: None,
            temp_millicelsius,
            policy,
            temp_crit,
            source: ThermalSource::ThermalZone,
        })
    }

    /// Check if this zone matches the filter pattern.
    fn matches_filter(&self, pattern: &str, case_insensitive: bool) -> bool {
        let fields = [
            self.name.as_str(),
            opt_str(&self.zone_type),
            opt_str(&self.label),
        ];
        matches_any(&fields, pattern, case_insensitive)
    }

    /// Temperature in degrees Celsius (for display).
    fn temp_celsius_x10(&self) -> Option<i32> {
        self.temp_millicelsius.map(|t| (t / 100) as i32)
    }

    /// Critical temperature in degrees Celsius x10.
    fn temp_crit_celsius_x10(&self) -> Option<i32> {
        self.temp_crit.map(|t| (t / 100) as i32)
    }

    /// Output as text.
    fn print_text(&self, verbose: bool, human: bool, zone_path: &str) {
        let mut w = TextWriter::new();

        // Use type as the primary identifier, fallback to zone name
        let sensor = self.zone_type.as_ref().map(|s| s.as_str()).unwrap_or(self.name.as_str());
        w.field_str(f::SENSOR, sensor);

        // For hwmon with labels, show the label (e.g., "Core 0")
        if let Some(ref label) = self.label {
            w.field_quoted(f::LABEL, label.as_str());
        }

        if let Some(temp_x10) = self.temp_celsius_x10() {
            format_temp_text(&mut w, f::TEMP, temp_x10, human);
        }

        if verbose {
            if let Some(crit_x10) = self.temp_crit_celsius_x10() {
                format_temp_text(&mut w, f::CRIT, crit_x10, human);
            }

            // Show trip points in verbose mode (for thermal zones only)
            if self.source == ThermalSource::ThermalZone {
                print_trip_points_text(&mut w, zone_path, human);
            }

            if let Some(ref policy) = self.policy {
                w.field_str(f::POLICY, policy.as_str());
            }

            w.field_str(f::SOURCE, self.source.as_str());
        }

        w.finish();
    }

    /// Write as JSON object.
    fn write_json(&self, w: &mut StreamingJsonWriter, verbose: bool, zone_path: &str) {
        w.array_object_begin();

        let sensor = self.zone_type.as_ref().map(|s| s.as_str()).unwrap_or(self.name.as_str());
        w.field_str(f::SENSOR, sensor);

        if let Some(ref label) = self.label {
            w.field_str(f::LABEL, label.as_str());
        }

        if let Some(temp) = self.temp_millicelsius {
            w.field_i64(f::TEMP_MILLICELSIUS, temp);
        }

        if verbose {
            w.field_str(f::NAME, self.name.as_str());
            if let Some(crit) = self.temp_crit {
                w.field_i64(f::TEMP_CRIT_MILLICELSIUS, crit);
            }

            // Include trip points in verbose JSON (for thermal zones only)
            if self.source == ThermalSource::ThermalZone {
                write_trip_points_json(w, zone_path);
            }

            if let Some(ref policy) = self.policy {
                w.field_str(f::POLICY, policy.as_str());
            }

            w.field_str(f::SOURCE, self.source.as_str());
        }

        w.array_object_end();
    }
}

/// A cooling device - fan, CPU frequency scaling, throttle alert, etc.
pub struct CoolingDevice {
    /// Device name (e.g., "cooling_device0")
    pub name: StackString<32>,
    /// Device type (e.g., "pwm-fan", "cpufreq-cpu0", "gpu-throttle-alert")
    pub device_type: StackString<64>,
    /// Current cooling state (0 = off/max freq, higher = more cooling/throttling)
    pub cur_state: u32,
    /// Maximum cooling state
    pub max_state: u32,
}

impl CoolingDevice {
    /// Read cooling device info from /sys/class/thermal.
    fn read(name: &str) -> Option<Self> {
        if !name.starts_with("cooling_device") {
            return None;
        }

        let base: StackString<128> = io::join_path(THERMAL_PATH, name);

        let type_path: StackString<128> = io::join_path(base.as_str(), "type");
        let cur_path: StackString<128> = io::join_path(base.as_str(), "cur_state");
        let max_path: StackString<128> = io::join_path(base.as_str(), "max_state");

        let device_type: StackString<64> = io::read_file_stack(type_path.as_str())?;
        let cur_state: u32 = io::read_file_parse(cur_path.as_str()).unwrap_or(0);
        let max_state: u32 = io::read_file_parse(max_path.as_str()).unwrap_or(0);

        Some(CoolingDevice {
            name: StackString::from_str(name),
            device_type,
            cur_state,
            max_state,
        })
    }

    /// Check if this device matches the filter pattern.
    fn matches_filter(&self, pattern: &str, case_insensitive: bool) -> bool {
        let fields = [
            self.name.as_str(),
            self.device_type.as_str(),
        ];
        matches_any(&fields, pattern, case_insensitive)
    }

    /// Output as text.
    fn print_text(&self) {
        let mut w = TextWriter::new();
        w.field_str(f::COOLING, self.device_type.as_str());

        if self.max_state > 0 {
            let mut state: StackString<16> = StackString::new();
            let mut buf = itoa::Buffer::new();
            state.push_str(buf.format(self.cur_state));
            state.push('/');
            state.push_str(buf.format(self.max_state));
            w.field_str(f::STATE, state.as_str());
        } else {
            w.field_u64(f::STATE, self.cur_state as u64);
        }

        w.finish();
    }

    /// Write as JSON object.
    fn write_json(&self, w: &mut StreamingJsonWriter) {
        w.array_object_begin();
        w.field_str(f::TYPE, self.device_type.as_str());
        w.field_u64(f::CUR_STATE, self.cur_state as u64);
        w.field_u64(f::MAX_STATE, self.max_state as u64);
        w.field_str(f::NAME, self.name.as_str());
        w.array_object_end();
    }
}

/// Read a single hwmon sensor.
struct HwmonSensor {
    /// Sensor name (e.g., "hwmon0", "hwmon0:2")
    name: StackString<32>,
    /// hwmon device type (e.g., "coretemp")
    zone_type: Option<StackString<64>>,
    /// Sensor label (e.g., "Core 0")
    label: Option<StackString<64>>,
    /// Current temperature in millidegrees Celsius
    temp_millicelsius: i64,
    /// Critical temperature threshold in millidegrees
    temp_crit: Option<i64>,
}

impl HwmonSensor {
    fn to_zone(self) -> ThermalZone {
        ThermalZone {
            name: self.name,
            zone_type: self.zone_type,
            label: self.label,
            temp_millicelsius: Some(self.temp_millicelsius),
            policy: None,
            temp_crit: self.temp_crit,
            source: ThermalSource::Hwmon,
        }
    }
}

/// Find critical temperature from trip points.
fn find_critical_trip_point(zone_path: &str) -> Option<i64> {
    for i in 0..16 {
        let mut type_file: StackString<128> = StackString::from_str(zone_path);
        type_file.push_str("/trip_point_");
        let mut buf = itoa::Buffer::new();
        type_file.push_str(buf.format(i));
        type_file.push_str("_type");

        let trip_type: Option<StackString<16>> = io::read_file_stack(type_file.as_str());
        if let Some(ref t) = trip_type {
            if t.as_str() == "critical" {
                let mut temp_file: StackString<128> = StackString::from_str(zone_path);
                temp_file.push_str("/trip_point_");
                temp_file.push_str(buf.format(i));
                temp_file.push_str("_temp");
                return io::read_file_parse(temp_file.as_str());
            }
        } else {
            break;
        }
    }
    None
}

/// Format temperature for text output.
fn format_temp_text(w: &mut TextWriter, name: &str, temp_x10: i32, human: bool) {
    let mut s: StackString<16> = StackString::new();
    push_fixed_point(&mut s, temp_x10 as i64, 1);
    if human {
        s.push('C');
    }
    w.field_str(name, s.as_str());
}

/// Print trip points for text output.
fn print_trip_points_text(w: &mut TextWriter, zone_path: &str, human: bool) {
    let mut trips: StackString<256> = StackString::new();
    let mut first = true;
    let mut consecutive_misses = 0;

    for i in 0..16 {
        let mut buf = itoa::Buffer::new();

        let mut type_file: StackString<128> = StackString::from_str(zone_path);
        type_file.push_str("/trip_point_");
        type_file.push_str(buf.format(i));
        type_file.push_str("_type");

        let mut temp_file: StackString<128> = StackString::from_str(zone_path);
        temp_file.push_str("/trip_point_");
        temp_file.push_str(buf.format(i));
        temp_file.push_str("_temp");

        let trip_type: Option<StackString<16>> = io::read_file_stack(type_file.as_str());
        let temp: Option<i64> = io::read_file_parse(temp_file.as_str());

        if let (Some(t), Some(temp_mc)) = (trip_type, temp) {
            if !first {
                trips.push(',');
            }
            first = false;
            trips.push_str(t.as_str());
            trips.push(':');

            push_fixed_point(&mut trips, temp_mc / 100, 1);
            if human {
                trips.push('C');
            }
            consecutive_misses = 0;
        } else {
            consecutive_misses += 1;
            if consecutive_misses >= 2 {
                break;
            }
        }
    }

    if !trips.is_empty() {
        w.field_str(f::TRIPS, trips.as_str());
    }
}

/// Write trip points to JSON.
fn write_trip_points_json(w: &mut StreamingJsonWriter, zone_path: &str) {
    let mut has_trips = false;
    let mut consecutive_misses = 0;

    // First pass: check if we have any trip points
    for i in 0..16 {
        let mut buf = itoa::Buffer::new();
        let mut type_file: StackString<128> = StackString::from_str(zone_path);
        type_file.push_str("/trip_point_");
        type_file.push_str(buf.format(i));
        type_file.push_str("_type");

        if io::path_exists(type_file.as_str()) {
            has_trips = true;
            break;
        }
    }

    if !has_trips {
        return;
    }

    w.field_array(f::TRIP_POINTS);

    for i in 0..16u32 {
        let mut buf = itoa::Buffer::new();

        let mut type_file: StackString<128> = StackString::from_str(zone_path);
        type_file.push_str("/trip_point_");
        type_file.push_str(buf.format(i));
        type_file.push_str("_type");

        let mut temp_file: StackString<128> = StackString::from_str(zone_path);
        temp_file.push_str("/trip_point_");
        temp_file.push_str(buf.format(i));
        temp_file.push_str("_temp");

        let trip_type: Option<StackString<16>> = io::read_file_stack(type_file.as_str());
        let temp: Option<i64> = io::read_file_parse(temp_file.as_str());

        if let (Some(t), Some(temp_mc)) = (trip_type, temp) {
            w.array_object_begin();
            w.field_u64(f::INDEX, i as u64);
            w.field_str(f::TYPE, t.as_str());
            w.field_i64(f::TEMP_MILLICELSIUS, temp_mc);
            w.array_object_end();
            consecutive_misses = 0;
        } else {
            consecutive_misses += 1;
            if consecutive_misses >= 2 {
                break;
            }
        }
    }

    w.end_field_array();
}

/// Check if thermal zones exist.
fn has_thermal_zones() -> bool {
    let mut found = false;
    io::for_each_dir_entry(THERMAL_PATH, |name| {
        if name.starts_with("thermal_zone") {
            found = true;
        }
    });
    found
}

/// Entry point for `kv thermal` subcommand.
pub fn run(opts: &GlobalOptions) -> i32 {
    let filter = opts.filter.as_ref().map(|s| s.as_str());
    let case_insensitive = opts.filter_case_insensitive;

    // Check if we have any thermal data
    let has_thermal = io::path_exists(THERMAL_PATH) && has_thermal_zones();
    let has_hwmon = io::path_exists(HWMON_PATH);

    if !has_thermal && !has_hwmon {
        if opts.json {
            let mut w = begin_kv_output_streaming(opts.pretty, "thermal");
            w.field_array("sensors");
            w.end_field_array();
            w.end_object();
            w.finish();
        } else {
            print::println("thermal: no temperature sensors found");
        }
        return 0;
    }

    if opts.json {
        let mut w = begin_kv_output_streaming(opts.pretty, "thermal");
        w.field_array("sensors");

        let mut count = 0;

        // First try thermal zones
        if has_thermal {
            io::for_each_dir_entry(THERMAL_PATH, |name| {
                if let Some(zone) = ThermalZone::read_thermal_zone(name) {
                    if let Some(pattern) = filter {
                        if !zone.matches_filter(pattern, case_insensitive) {
                            return;
                        }
                    }
                    let zone_path: StackString<128> = io::join_path(THERMAL_PATH, name);
                    zone.write_json(&mut w, opts.verbose, zone_path.as_str());
                    count += 1;
                }
            });
        }

        // Fall back to hwmon if no thermal zones
        if count == 0 && has_hwmon {
            io::for_each_dir_entry(HWMON_PATH, |hwmon_name| {
                let hwmon_path: StackString<128> = io::join_path(HWMON_PATH, hwmon_name);
                let name_path: StackString<128> = io::join_path(hwmon_path.as_str(), "name");
                let hwmon_type: Option<StackString<64>> = io::read_file_stack(name_path.as_str());

                // Check up to 16 temperature inputs
                for i in 1..=16u32 {
                    let mut buf = itoa::Buffer::new();

                    let mut temp_file: StackString<128> = StackString::from_str(hwmon_path.as_str());
                    temp_file.push_str("/temp");
                    temp_file.push_str(buf.format(i));
                    temp_file.push_str("_input");

                    if let Some(temp) = io::read_file_parse::<i64>(temp_file.as_str()) {
                        // Read optional label
                        let mut label_file: StackString<128> = StackString::from_str(hwmon_path.as_str());
                        label_file.push_str("/temp");
                        label_file.push_str(buf.format(i));
                        label_file.push_str("_label");
                        let label: Option<StackString<64>> = io::read_file_stack(label_file.as_str());

                        // Read optional critical temp
                        let mut crit_file: StackString<128> = StackString::from_str(hwmon_path.as_str());
                        crit_file.push_str("/temp");
                        crit_file.push_str(buf.format(i));
                        crit_file.push_str("_crit");
                        let temp_crit: Option<i64> = io::read_file_parse(crit_file.as_str());

                        // Create sensor name
                        let sensor_name: StackString<32> = if i == 1 {
                            StackString::from_str(hwmon_name)
                        } else {
                            let mut name: StackString<32> = StackString::from_str(hwmon_name);
                            name.push(':');
                            name.push_str(buf.format(i));
                            name
                        };

                        let sensor = HwmonSensor {
                            name: sensor_name,
                            zone_type: hwmon_type.clone(),
                            label,
                            temp_millicelsius: temp,
                            temp_crit,
                        };
                        let zone = sensor.to_zone();

                        if let Some(pattern) = filter {
                            if !zone.matches_filter(pattern, case_insensitive) {
                                continue;
                            }
                        }

                        zone.write_json(&mut w, opts.verbose, "");
                        count += 1;
                    }
                }
            });
        }

        w.end_field_array();

        // Include cooling devices in verbose mode
        if opts.verbose {
            let mut has_cooling = false;
            io::for_each_dir_entry(THERMAL_PATH, |name| {
                if name.starts_with("cooling_device") {
                    if !has_cooling {
                        w.field_array(f::COOLING);
                        has_cooling = true;
                    }
                    if let Some(dev) = CoolingDevice::read(name) {
                        if let Some(pattern) = filter {
                            if !dev.matches_filter(pattern, case_insensitive) {
                                return;
                            }
                        }
                        dev.write_json(&mut w);
                    }
                }
            });
            if has_cooling {
                w.end_field_array();
            }
        }

        w.end_object();
        w.finish();
    } else {
        let mut count = 0;

        // First try thermal zones
        if has_thermal {
            io::for_each_dir_entry(THERMAL_PATH, |name| {
                if let Some(zone) = ThermalZone::read_thermal_zone(name) {
                    if let Some(pattern) = filter {
                        if !zone.matches_filter(pattern, case_insensitive) {
                            return;
                        }
                    }
                    let zone_path: StackString<128> = io::join_path(THERMAL_PATH, name);
                    zone.print_text(opts.verbose, opts.human, zone_path.as_str());
                    count += 1;
                }
            });
        }

        // Fall back to hwmon if no thermal zones
        if count == 0 && has_hwmon {
            io::for_each_dir_entry(HWMON_PATH, |hwmon_name| {
                let hwmon_path: StackString<128> = io::join_path(HWMON_PATH, hwmon_name);
                let name_path: StackString<128> = io::join_path(hwmon_path.as_str(), "name");
                let hwmon_type: Option<StackString<64>> = io::read_file_stack(name_path.as_str());

                // Check up to 16 temperature inputs
                for i in 1..=16u32 {
                    let mut buf = itoa::Buffer::new();

                    let mut temp_file: StackString<128> = StackString::from_str(hwmon_path.as_str());
                    temp_file.push_str("/temp");
                    temp_file.push_str(buf.format(i));
                    temp_file.push_str("_input");

                    if let Some(temp) = io::read_file_parse::<i64>(temp_file.as_str()) {
                        // Read optional label
                        let mut label_file: StackString<128> = StackString::from_str(hwmon_path.as_str());
                        label_file.push_str("/temp");
                        label_file.push_str(buf.format(i));
                        label_file.push_str("_label");
                        let label: Option<StackString<64>> = io::read_file_stack(label_file.as_str());

                        // Read optional critical temp
                        let mut crit_file: StackString<128> = StackString::from_str(hwmon_path.as_str());
                        crit_file.push_str("/temp");
                        crit_file.push_str(buf.format(i));
                        crit_file.push_str("_crit");
                        let temp_crit: Option<i64> = io::read_file_parse(crit_file.as_str());

                        // Create sensor name
                        let sensor_name: StackString<32> = if i == 1 {
                            StackString::from_str(hwmon_name)
                        } else {
                            let mut name: StackString<32> = StackString::from_str(hwmon_name);
                            name.push(':');
                            name.push_str(buf.format(i));
                            name
                        };

                        let sensor = HwmonSensor {
                            name: sensor_name,
                            zone_type: hwmon_type.clone(),
                            label,
                            temp_millicelsius: temp,
                            temp_crit,
                        };
                        let zone = sensor.to_zone();

                        if let Some(pattern) = filter {
                            if !zone.matches_filter(pattern, case_insensitive) {
                                continue;
                            }
                        }

                        zone.print_text(opts.verbose, opts.human, "");
                        count += 1;
                    }
                }
            });
        }

        // Print cooling devices in verbose mode
        if opts.verbose {
            io::for_each_dir_entry(THERMAL_PATH, |name| {
                if let Some(dev) = CoolingDevice::read(name) {
                    if let Some(pattern) = filter {
                        if !dev.matches_filter(pattern, case_insensitive) {
                            return;
                        }
                    }
                    dev.print_text();
                }
            });
        }

        if count == 0 {
            if filter.is_some() {
                print::println("thermal: no matching sensors");
            } else {
                print::println("thermal: no temperature sensors found");
            }
        }
    }

    0
}

/// Write thermal sensors to JSON writer (for snapshot).
#[cfg(feature = "snapshot")]
pub fn write_snapshot(w: &mut StreamingJsonWriter, verbose: bool) {
    let has_thermal = io::path_exists(THERMAL_PATH) && has_thermal_zones();
    let has_hwmon = io::path_exists(HWMON_PATH);

    if !has_thermal && !has_hwmon {
        return;
    }

    w.key("thermal");
    w.begin_array();

    let mut count = 0;

    // First try thermal zones
    if has_thermal {
        io::for_each_dir_entry(THERMAL_PATH, |name| {
            if let Some(zone) = ThermalZone::read_thermal_zone(name) {
                let zone_path: StackString<128> = io::join_path(THERMAL_PATH, name);
                zone.write_json(w, verbose, zone_path.as_str());
                count += 1;
            }
        });
    }

    // Fall back to hwmon if no thermal zones
    if count == 0 && has_hwmon {
        io::for_each_dir_entry(HWMON_PATH, |hwmon_name| {
            let hwmon_path: StackString<128> = io::join_path(HWMON_PATH, hwmon_name);
            let name_path: StackString<128> = io::join_path(hwmon_path.as_str(), "name");
            let hwmon_type: Option<StackString<64>> = io::read_file_stack(name_path.as_str());

            for i in 1..=16u32 {
                let mut buf = itoa::Buffer::new();

                let mut temp_file: StackString<128> = StackString::from_str(hwmon_path.as_str());
                temp_file.push_str("/temp");
                temp_file.push_str(buf.format(i));
                temp_file.push_str("_input");

                if let Some(temp) = io::read_file_parse::<i64>(temp_file.as_str()) {
                    let mut label_file: StackString<128> = StackString::from_str(hwmon_path.as_str());
                    label_file.push_str("/temp");
                    label_file.push_str(buf.format(i));
                    label_file.push_str("_label");
                    let label: Option<StackString<64>> = io::read_file_stack(label_file.as_str());

                    let mut crit_file: StackString<128> = StackString::from_str(hwmon_path.as_str());
                    crit_file.push_str("/temp");
                    crit_file.push_str(buf.format(i));
                    crit_file.push_str("_crit");
                    let temp_crit: Option<i64> = io::read_file_parse(crit_file.as_str());

                    let sensor_name: StackString<32> = if i == 1 {
                        StackString::from_str(hwmon_name)
                    } else {
                        let mut name: StackString<32> = StackString::from_str(hwmon_name);
                        name.push(':');
                        name.push_str(buf.format(i));
                        name
                    };

                    let sensor = HwmonSensor {
                        name: sensor_name,
                        zone_type: hwmon_type.clone(),
                        label,
                        temp_millicelsius: temp,
                        temp_crit,
                    };
                    sensor.to_zone().write_json(w, verbose, "");
                }
            }
        });
    }

    w.end_array();
}
