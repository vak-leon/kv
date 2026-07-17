//! Device tree information from /sys/firmware/devicetree/base.
//!
//! The devicetree is the primary hardware description mechanism on ARM,
//! AArch64, and RISC-V systems. It's a hierarchical tree of nodes that
//! describe the hardware topology.
//!
//! Unlike other subcommands, dt supports filtering because the full tree
//! can have hundreds of nodes. Common use cases:
//! - Quick board identification (default: show root model/compatible)
//! - Find disabled devices (common debugging task)
//! - Search for specific device types
//! - Inspect a specific node path

#![cfg(any(target_arch = "arm", target_arch = "aarch64", target_arch = "riscv64", target_arch = "powerpc64", target_arch = "mips"))]

use crate::cli::{ExtraArgs, GlobalOptions};
use crate::fields::dt as f;
use crate::filter::matches_any;
use crate::io;
use crate::json::{begin_kv_output_streaming, StreamingJsonWriter};
use crate::print::{self, TextWriter};
use crate::stack::StackString;

const DT_BASE_PATH: &str = "/sys/firmware/devicetree/base";

// =============================================================================
// Input Safety Limits
// =============================================================================

/// Maximum recursion depth for traversing devicetree (defense against stack overflow).
/// Real devicetrees rarely exceed 10-15 levels, so 64 is generous.
const MAX_RECURSION_DEPTH: usize = 64;

/// Maximum number of nodes to process (defense against symlink loops or huge trees).
const MAX_NODE_COUNT: usize = 4096;

/// Maximum property file size to read (64 KiB - defense against large file attacks).
const MAX_PROPERTY_SIZE: u64 = 64 * 1024;

/// Maximum number of properties to output per node.
const MAX_PROPERTIES_PER_NODE: usize = 64;

/// Options specific to the dt subcommand.
#[derive(Default)]
pub struct DtOptions {
    /// Show only disabled nodes
    pub disabled_only: bool,
    /// Specific node path to inspect
    pub node_path: Option<StackString<256>>,
}

impl DtOptions {
    /// Parse dt-specific options from remaining arguments.
    pub fn parse(args: &ExtraArgs) -> Self {
        let mut opts = DtOptions::default();

        for arg in args.iter() {
            match arg {
                "-d" | "--disabled" => {
                    opts.disabled_only = true;
                }
                s if s.starts_with('/') => {
                    opts.node_path = Some(StackString::from_str(s));
                }
                _ => {}
            }
        }

        opts
    }
}

/// Key properties of a DT node (for filtering and basic display).
struct DtNodeInfo {
    path: StackString<256>,
    name: StackString<64>,
    compatible: Option<StackString<512>>,
    status: Option<StackString<512>>,
    model: Option<StackString<512>>,
}

impl DtNodeInfo {
    /// Check if this node is disabled.
    fn is_disabled(&self) -> bool {
        if let Some(ref status) = self.status {
            let s = status.as_str();
            !s.eq_ignore_ascii_case("okay") && !s.eq_ignore_ascii_case("ok")
        } else {
            false
        }
    }

    /// Check if this node matches the filter pattern.
    fn matches_filter(&self, pattern: &str, case_insensitive: bool) -> bool {
        let compat = self.compatible.as_ref().map(|s| s.as_str()).unwrap_or("");
        let fields = [self.path.as_str(), compat];
        matches_any(&fields, pattern, case_insensitive)
    }
}

/// Read a property file and try to interpret it as a readable string.
/// Skips symlinks and files larger than MAX_PROPERTY_SIZE for safety.
fn read_property(path: &str) -> Option<StackString<512>> {
    // Skip symlinks - they could point outside the DT base
    if io::is_symlink(path) {
        return None;
    }

    // Skip files that are too large
    if let Some(size) = io::file_size(path) {
        if size > MAX_PROPERTY_SIZE {
            return None;
        }
    }

    // Read the raw bytes - properties like `reg` are binary, so no UTF-8
    // filtering or trimming here, since that would corrupt them (or drop them
    // before the hex fallback below ever ran).
    let mut buf = [0u8; 4096];
    let len = io::read_file_raw(path, &mut buf)?;
    let bytes = &buf[..len];

    // An empty property is a boolean flag (present = true)
    if bytes.is_empty() {
        return Some(StackString::new());
    }

    // Try to interpret as string(s) - devicetree strings are null-terminated
    let is_stringy = bytes.iter().all(|&b| {
        b == 0 || (b >= 0x20 && b < 0x7f) || b == b'\n' || b == b'\t'
    });

    if is_stringy {
        // Split by null bytes and join with ", "
        let mut result: StackString<512> = StackString::new();
        let mut first = true;
        for part in bytes.split(|&b| b == 0) {
            if part.is_empty() {
                continue;
            }
            if let Ok(s) = core::str::from_utf8(part) {
                if !first {
                    result.push_str(", ");
                }
                result.push_str(s);
                first = false;
            }
        }
        if !result.as_str().is_empty() {
            return Some(result);
        }
    }

    // Fall back to hex for binary data (limit to 32 bytes)
    let mut result: StackString<512> = StackString::new();
    let limit = if bytes.len() > 32 { 32 } else { bytes.len() };
    for (i, &b) in bytes[..limit].iter().enumerate() {
        if i > 0 {
            result.push(' ');
        }
        result.push(io::HexNibble::hex_hi(b));
        result.push(io::HexNibble::hex_lo(b));
    }
    if bytes.len() > 32 {
        result.push_str("...");
    }
    Some(result)
}

#[cfg(test)]
mod property_tests {
    use super::*;

    fn tmp_prop(name: &str, content: &[u8]) -> std::string::String {
        let mut p = std::env::temp_dir();
        p.push(std::format!("kv-dt-test-{}-{}", std::process::id(), name));
        std::fs::write(&p, content).unwrap();
        p.to_str().unwrap().to_string()
    }

    #[test]
    fn string_property_joins_null_separated_parts() {
        let p = tmp_prop("compat", b"brcm,bcm2711\0raspberrypi,4-model-b\0");
        let v = read_property(&p);
        std::fs::remove_file(&p).unwrap();
        assert_eq!(v.as_deref(), Some("brcm,bcm2711, raspberrypi,4-model-b"));
    }

    #[test]
    fn binary_property_falls_back_to_hex() {
        // A typical `reg` cell: high bytes make it non-UTF-8
        let p = tmp_prop("reg", &[0xfe, 0x20, 0x10, 0x00]);
        let v = read_property(&p);
        std::fs::remove_file(&p).unwrap();
        assert_eq!(v.as_deref(), Some("fe 20 10 00"));
    }

    #[test]
    fn empty_property_is_a_boolean_flag() {
        let p = tmp_prop("flag", b"");
        let v = read_property(&p);
        std::fs::remove_file(&p).unwrap();
        assert_eq!(v.as_deref(), Some(""));
    }
}

/// Sanitize a relative path, rejecting any path traversal attempts.
fn sanitize_relative_path(base_path: &str, relative_path: &str) -> Option<StackString<512>> {
    if relative_path == "/" {
        return Some(StackString::from_str(base_path));
    }

    let clean = relative_path.trim_start_matches('/');

    // Reject any path with ".." components
    for component in clean.split('/') {
        if component == ".." || component.is_empty() && clean.contains("//") {
            return None;
        }
    }

    Some(io::join_path(base_path, clean))
}

/// Read key properties from a DT node directory.
fn read_node_info(base_path: &str, relative_path: &str) -> Option<DtNodeInfo> {
    let full_path = sanitize_relative_path(base_path, relative_path)?;

    if !io::is_dir(full_path.as_str()) {
        return None;
    }

    let name: StackString<64> = if relative_path == "/" {
        StackString::from_str("/")
    } else {
        let parts: StackString<256> = StackString::from_str(relative_path);
        let last = parts.as_str().rsplit('/').next().unwrap_or(relative_path);
        StackString::from_str(last)
    };

    // Read key properties
    let compat_path: StackString<512> = io::join_path(full_path.as_str(), "compatible");
    let status_path: StackString<512> = io::join_path(full_path.as_str(), "status");
    let model_path: StackString<512> = io::join_path(full_path.as_str(), "model");

    Some(DtNodeInfo {
        path: if relative_path.is_empty() {
            StackString::from_str("/")
        } else {
            StackString::from_str(relative_path)
        },
        name,
        compatible: read_property(compat_path.as_str()),
        status: read_property(status_path.as_str()),
        model: read_property(model_path.as_str()),
    })
}

/// Output a node's properties inline (reads from disk during output).
fn output_properties_text(full_path: &str) {
    let mut count = 0;
    io::for_each_dir_entry(full_path, |name| {
        if count >= MAX_PROPERTIES_PER_NODE {
            return;
        }
        if name == "name" {
            return; // Redundant with node name
        }

        let prop_path: StackString<512> = io::join_path(full_path, name);

        // Skip directories and symlinks
        if !io::is_file(prop_path.as_str()) {
            return;
        }

        if let Some(value) = read_property(prop_path.as_str()) {
            // Quote values that contain spaces or special chars
            if value.as_str().contains(' ') || value.as_str().contains(',') || value.as_str().is_empty() {
                print::print("  ");
                print::print(name);
                print::print("=\"");
                print::print(value.as_str());
                print::println("\"");
            } else {
                print::print("  ");
                print::print(name);
                print::print("=");
                print::println(value.as_str());
            }
            count += 1;
        }
    });
}

/// Output a node's properties as JSON (reads from disk during output).
fn output_properties_json(w: &mut StreamingJsonWriter, full_path: &str) {
    w.field_object(f::PROPERTIES);

    let mut count = 0;
    io::for_each_dir_entry(full_path, |name| {
        if count >= MAX_PROPERTIES_PER_NODE {
            return;
        }
        if name == "name" {
            return;
        }

        let prop_path: StackString<512> = io::join_path(full_path, name);

        if !io::is_file(prop_path.as_str()) {
            return;
        }

        if let Some(value) = read_property(prop_path.as_str()) {
            w.field_str(name, value.as_str());
            count += 1;
        }
    });

    w.end_field_object();
}

/// Counter for limiting nodes during traversal.
struct NodeCounter {
    count: usize,
}

impl NodeCounter {
    fn new() -> Self {
        Self { count: 0 }
    }

    fn increment(&mut self) -> bool {
        if self.count >= MAX_NODE_COUNT {
            false
        } else {
            self.count += 1;
            true
        }
    }
}

/// Recursively count nodes (for summary).
fn count_nodes_recursive(base_path: &str, relative_path: &str, depth: usize) -> usize {
    if depth > MAX_RECURSION_DEPTH {
        return 0;
    }

    let full_path = match sanitize_relative_path(base_path, relative_path) {
        Some(p) => p,
        None => return 0,
    };

    if !io::is_dir(full_path.as_str()) {
        return 0;
    }

    let mut count = 1; // This node

    io::for_each_dir_entry(full_path.as_str(), |name| {
        if count >= MAX_NODE_COUNT {
            return;
        }

        let child_full_path: StackString<512> = io::join_path(full_path.as_str(), name);

        // Skip symlinks
        if io::is_symlink(child_full_path.as_str()) {
            return;
        }

        // Only recurse into directories
        if !io::is_dir(child_full_path.as_str()) {
            return;
        }

        let child_path: StackString<512> = if relative_path == "/" {
            let mut p: StackString<512> = StackString::new();
            p.push('/');
            p.push_str(name);
            p
        } else {
            let mut p: StackString<512> = StackString::new();
            p.push_str(relative_path);
            p.push('/');
            p.push_str(name);
            p
        };

        count += count_nodes_recursive(base_path, child_path.as_str(), depth + 1);
    });

    count
}

/// Recursively traverse and output nodes (streaming).
fn traverse_and_output_text(
    base_path: &str,
    relative_path: &str,
    depth: usize,
    counter: &mut NodeCounter,
    opts: &GlobalOptions,
    dt_opts: &DtOptions,
) {
    if depth > MAX_RECURSION_DEPTH || !counter.increment() {
        return;
    }

    let full_path = match sanitize_relative_path(base_path, relative_path) {
        Some(p) => p,
        None => return,
    };

    if !io::is_dir(full_path.as_str()) {
        return;
    }

    // Read and potentially output this node
    if let Some(info) = read_node_info(base_path, relative_path) {
        // Apply filters
        let mut skip = false;
        if dt_opts.disabled_only && !info.is_disabled() {
            skip = true;
        }
        if let Some(ref pattern) = opts.filter {
            if !info.matches_filter(pattern.as_str(), opts.filter_case_insensitive) {
                skip = true;
            }
        }

        if !skip {
            let mut w = TextWriter::new();
            w.field_str(f::PATH, info.path.as_str());
            if let Some(ref compat) = info.compatible {
                w.field_quoted(f::COMPATIBLE, compat.as_str());
            }
            if let Some(ref status) = info.status {
                w.field_str(f::STATUS, status.as_str());
            }
            if opts.verbose {
                if let Some(ref model) = info.model {
                    w.field_quoted(f::MODEL, model.as_str());
                }
            }
            w.finish();
        }
    }

    // Recurse into children
    io::for_each_dir_entry(full_path.as_str(), |name| {
        let child_full_path: StackString<512> = io::join_path(full_path.as_str(), name);

        if io::is_symlink(child_full_path.as_str()) {
            return;
        }

        if !io::is_dir(child_full_path.as_str()) {
            return;
        }

        let child_path: StackString<512> = if relative_path == "/" {
            let mut p: StackString<512> = StackString::new();
            p.push('/');
            p.push_str(name);
            p
        } else {
            let mut p: StackString<512> = StackString::new();
            p.push_str(relative_path);
            p.push('/');
            p.push_str(name);
            p
        };

        traverse_and_output_text(base_path, child_path.as_str(), depth + 1, counter, opts, dt_opts);
    });
}

/// Recursively traverse and output nodes as JSON (streaming).
fn traverse_and_output_json(
    w: &mut StreamingJsonWriter,
    base_path: &str,
    relative_path: &str,
    depth: usize,
    counter: &mut NodeCounter,
    opts: &GlobalOptions,
    dt_opts: &DtOptions,
) {
    if depth > MAX_RECURSION_DEPTH || !counter.increment() {
        return;
    }

    let full_path = match sanitize_relative_path(base_path, relative_path) {
        Some(p) => p,
        None => return,
    };

    if !io::is_dir(full_path.as_str()) {
        return;
    }

    // Read and potentially output this node
    if let Some(info) = read_node_info(base_path, relative_path) {
        // Apply filters
        let mut skip = false;
        if dt_opts.disabled_only && !info.is_disabled() {
            skip = true;
        }
        if let Some(ref pattern) = opts.filter {
            if !info.matches_filter(pattern.as_str(), opts.filter_case_insensitive) {
                skip = true;
            }
        }

        if !skip {
            w.array_object_begin();
            w.field_str(f::PATH, info.path.as_str());
            w.field_str(f::NAME, info.name.as_str());
            w.field_str_opt(f::COMPATIBLE, info.compatible.as_ref().map(|s| s.as_str()));
            w.field_str_opt(f::STATUS, info.status.as_ref().map(|s| s.as_str()));

            if opts.verbose {
                output_properties_json(w, full_path.as_str());
            }

            w.array_object_end();
        }
    }

    // Recurse into children
    io::for_each_dir_entry(full_path.as_str(), |name| {
        let child_full_path: StackString<512> = io::join_path(full_path.as_str(), name);

        if io::is_symlink(child_full_path.as_str()) {
            return;
        }

        if !io::is_dir(child_full_path.as_str()) {
            return;
        }

        let child_path: StackString<512> = if relative_path == "/" {
            let mut p: StackString<512> = StackString::new();
            p.push('/');
            p.push_str(name);
            p
        } else {
            let mut p: StackString<512> = StackString::new();
            p.push_str(relative_path);
            p.push('/');
            p.push_str(name);
            p
        };

        traverse_and_output_json(w, base_path, child_path.as_str(), depth + 1, counter, opts, dt_opts);
    });
}

/// Entry point for `kv dt` subcommand.
pub fn run(opts: &GlobalOptions, args: &ExtraArgs) -> i32 {
    let dt_opts = DtOptions::parse(args);

    if !io::path_exists(DT_BASE_PATH) {
        if opts.json {
            let mut w = begin_kv_output_streaming(opts.pretty, "dt");
            w.key("data");
            w.value_null();
            w.field_str("error", "devicetree not found");
            w.end_object();
            w.finish();
        } else {
            print::print("dt: devicetree not found (");
            print::print(DT_BASE_PATH);
            print::println(")");
        }
        return 0;
    }

    // Mode 1: Specific node path
    if let Some(ref node_path) = dt_opts.node_path {
        return run_single_node(opts, node_path.as_str());
    }

    // Mode 2: Filtered list (disabled or global filter pattern)
    if dt_opts.disabled_only || opts.filter.is_some() {
        return run_filtered(opts, &dt_opts);
    }

    // Mode 3: Default - show root summary (or full list with -v)
    if opts.verbose {
        return run_full_list(opts, &dt_opts);
    }

    run_summary(opts)
}

/// Show summary: root node info + node count.
fn run_summary(opts: &GlobalOptions) -> i32 {
    let root = read_node_info(DT_BASE_PATH, "/");
    let count = count_nodes_recursive(DT_BASE_PATH, "/", 0);

    if opts.json {
        let mut w = begin_kv_output_streaming(opts.pretty, "dt");
        w.field_object("data");

        if let Some(ref node) = root {
            w.field_str_opt(f::MODEL, node.model.as_ref().map(|s| s.as_str()));
            w.field_str_opt(f::COMPATIBLE, node.compatible.as_ref().map(|s| s.as_str()));
        }
        w.field_u64(f::NODE_COUNT, count as u64);

        w.end_field_object();
        w.end_object();
        w.finish();
    } else {
        if let Some(ref node) = root {
            if let Some(ref model) = node.model {
                print::print("MODEL=\"");
                print::print(model.as_str());
                print::println("\"");
            }
            if let Some(ref compat) = node.compatible {
                print::print("COMPATIBLE=\"");
                print::print(compat.as_str());
                print::println("\"");
            }
        }
        print::print("NODES=");
        print::println_u64(count as u64);
        print::println_empty();
        print::println("Use -v for full list, -f <pattern> to search, -d for disabled nodes");
    }

    0
}

/// Show a single node in detail.
fn run_single_node(opts: &GlobalOptions, node_path: &str) -> i32 {
    let full_path = match sanitize_relative_path(DT_BASE_PATH, node_path) {
        Some(p) => p,
        None => {
            if opts.json {
                let mut w = begin_kv_output_streaming(opts.pretty, "dt");
                w.key("data");
                w.value_null();
                w.field_str("error", "invalid path");
                w.end_object();
                w.finish();
            } else {
                print::print("dt: invalid path: ");
                print::println(node_path);
            }
            return 0;
        }
    };

    let info = match read_node_info(DT_BASE_PATH, node_path) {
        Some(n) => n,
        None => {
            if opts.json {
                let mut w = begin_kv_output_streaming(opts.pretty, "dt");
                w.key("data");
                w.value_null();
                w.field_str("error", "node not found");
                w.end_object();
                w.finish();
            } else {
                print::print("dt: node not found: ");
                print::println(node_path);
            }
            return 0;
        }
    };

    if opts.json {
        let mut w = begin_kv_output_streaming(opts.pretty, "dt");
        w.field_object("data");
        w.field_str(f::PATH, info.path.as_str());
        w.field_str(f::NAME, info.name.as_str());

        output_properties_json(&mut w, full_path.as_str());

        w.end_field_object();
        w.end_object();
        w.finish();
    } else {
        print::print("PATH=");
        print::println(info.path.as_str());
        output_properties_text(full_path.as_str());
    }

    0
}

/// Show filtered list of nodes.
fn run_filtered(opts: &GlobalOptions, dt_opts: &DtOptions) -> i32 {
    let mut counter = NodeCounter::new();

    if opts.json {
        let mut w = begin_kv_output_streaming(opts.pretty, "dt");
        w.field_array("data");

        traverse_and_output_json(&mut w, DT_BASE_PATH, "/", 0, &mut counter, opts, dt_opts);

        w.end_field_array();
        w.end_object();
        w.finish();
    } else {
        traverse_and_output_text(DT_BASE_PATH, "/", 0, &mut counter, opts, dt_opts);

        if counter.count == 0 {
            print::println("dt: no matching nodes");
        } else {
            print::println_empty();
            print::print("(");
            print::print_u64(counter.count as u64);
            print::println(" nodes)");
        }
    }

    0
}

/// Show full list of all nodes.
fn run_full_list(opts: &GlobalOptions, dt_opts: &DtOptions) -> i32 {
    let mut counter = NodeCounter::new();

    if opts.json {
        let mut w = begin_kv_output_streaming(opts.pretty, "dt");
        w.field_array("data");

        traverse_and_output_json(&mut w, DT_BASE_PATH, "/", 0, &mut counter, opts, dt_opts);

        w.end_field_array();
        w.end_object();
        w.finish();
    } else {
        traverse_and_output_text(DT_BASE_PATH, "/", 0, &mut counter, opts, dt_opts);
    }

    0
}

/// Write DT nodes to JSON writer (for snapshot).
#[cfg(feature = "snapshot")]
pub fn write_snapshot(w: &mut StreamingJsonWriter, verbose: bool) {
    if !io::path_exists(DT_BASE_PATH) {
        return;
    }

    w.key("dt");
    w.begin_array();

    let mut counter = NodeCounter::new();
    traverse_and_output_json_snapshot(w, DT_BASE_PATH, "/", 0, &mut counter, verbose);

    w.end_array();
}

/// Simplified JSON traversal for snapshot (no filtering).
#[cfg(feature = "snapshot")]
fn traverse_and_output_json_snapshot(
    w: &mut StreamingJsonWriter,
    base_path: &str,
    relative_path: &str,
    depth: usize,
    counter: &mut NodeCounter,
    verbose: bool,
) {
    if depth > MAX_RECURSION_DEPTH || !counter.increment() {
        return;
    }

    let full_path = match sanitize_relative_path(base_path, relative_path) {
        Some(p) => p,
        None => return,
    };

    if !io::is_dir(full_path.as_str()) {
        return;
    }

    if let Some(info) = read_node_info(base_path, relative_path) {
        w.array_object_begin();
        w.field_str(f::PATH, info.path.as_str());
        w.field_str(f::NAME, info.name.as_str());
        w.field_str_opt(f::COMPATIBLE, info.compatible.as_ref().map(|s| s.as_str()));
        w.field_str_opt(f::STATUS, info.status.as_ref().map(|s| s.as_str()));

        if verbose {
            output_properties_json(w, full_path.as_str());
        }

        w.array_object_end();
    }

    io::for_each_dir_entry(full_path.as_str(), |name| {
        let child_full_path: StackString<512> = io::join_path(full_path.as_str(), name);

        if io::is_symlink(child_full_path.as_str()) {
            return;
        }

        if !io::is_dir(child_full_path.as_str()) {
            return;
        }

        let child_path: StackString<512> = if relative_path == "/" {
            let mut p: StackString<512> = StackString::new();
            p.push('/');
            p.push_str(name);
            p
        } else {
            let mut p: StackString<512> = StackString::new();
            p.push_str(relative_path);
            p.push('/');
            p.push_str(name);
            p
        };

        traverse_and_output_json_snapshot(w, base_path, child_path.as_str(), depth + 1, counter, verbose);
    });
}
