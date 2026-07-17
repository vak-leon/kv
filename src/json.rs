//! Streaming JSON serialization without heap allocation.
//!
//! This module provides a JSON writer that writes directly to stdout,
//! avoiding any heap allocation. It handles proper escaping and supports
//! both compact and pretty-printed output.
//!
//! The escape handling is modeled after serde_json's approach: a static lookup
//! table for fast character classification, then handle each escape type.
//!
//! Uses itoa for integer formatting to avoid core::fmt bloat.

use crate::io::HexNibble;
use crate::print;

/// Escape types for JSON string escaping.
/// Inspired by serde_json's approach - use a lookup table for speed.
#[derive(Clone, Copy)]
enum EscapeType {
    /// No escaping needed
    None,
    /// Backspace -> \b
    Backspace,
    /// Tab -> \t
    Tab,
    /// Newline -> \n
    Newline,
    /// Form feed -> \f (rare but valid)
    FormFeed,
    /// Carriage return -> \r
    CarriageReturn,
    /// Quote -> \"
    Quote,
    /// Backslash -> \\
    Backslash,
    /// Other control character -> \uXXXX
    Unicode,
}

/// Lookup table for ASCII characters (0-127).
/// Characters >= 128 are passed through as-is (valid UTF-8).
static ESCAPE_TABLE: [EscapeType; 128] = {
    use EscapeType::*;
    [
        // 0x00 - 0x1F: Control characters
        Unicode, Unicode, Unicode, Unicode, Unicode, Unicode, Unicode, Unicode, // 0x00-0x07
        Backspace, Tab, Newline, Unicode, FormFeed, CarriageReturn, Unicode, Unicode, // 0x08-0x0F
        Unicode, Unicode, Unicode, Unicode, Unicode, Unicode, Unicode, Unicode, // 0x10-0x17
        Unicode, Unicode, Unicode, Unicode, Unicode, Unicode, Unicode, Unicode, // 0x18-0x1F
        // 0x20 - 0x7F: Printable ASCII
        None, None, Quote, None, None, None, None, None, // 0x20-0x27 (space, !, ", #, $, %, &, ')
        None, None, None, None, None, None, None, None, // 0x28-0x2F
        None, None, None, None, None, None, None, None, // 0x30-0x37
        None, None, None, None, None, None, None, None, // 0x38-0x3F
        None, None, None, None, None, None, None, None, // 0x40-0x47
        None, None, None, None, None, None, None, None, // 0x48-0x4F
        None, None, None, None, None, None, None, None, // 0x50-0x57
        None, None, None, None, Backslash, None, None, None, // 0x58-0x5F (includes \)
        None, None, None, None, None, None, None, None, // 0x60-0x67
        None, None, None, None, None, None, None, None, // 0x68-0x6F
        None, None, None, None, None, None, None, None, // 0x70-0x77
        None, None, None, None, None, None, None, Unicode, // 0x78-0x7F (DEL is control)
    ]
};

/// JSON writer that streams directly to stdout.
/// This avoids heap allocation by writing output immediately.
pub struct StreamingJsonWriter {
    pretty: bool,
    indent_level: usize,
    needs_comma: bool,
}

impl StreamingJsonWriter {
    /// Create a new streaming JSON writer.
    pub fn new(pretty: bool) -> Self {
        Self {
            pretty,
            indent_level: 0,
            needs_comma: false,
        }
    }

    /// Finish writing (just outputs a newline).
    pub fn finish(self) {
        print::println_empty();
    }

    fn write_indent(&mut self) {
        if self.pretty {
            for _ in 0..self.indent_level {
                print::print("  ");
            }
        }
    }

    fn write_newline(&mut self) {
        if self.pretty {
            print::print("\n");
        }
    }

    fn write_separator(&mut self) {
        if self.needs_comma {
            print::print(",");
            self.write_newline();
        }
        self.needs_comma = false;
    }

    /// Begin a JSON object `{`.
    pub fn begin_object(&mut self) {
        self.write_separator();
        self.write_indent();
        print::print("{");
        self.write_newline();
        self.indent_level += 1;
        self.needs_comma = false;
    }

    /// End a JSON object `}`.
    pub fn end_object(&mut self) {
        self.write_newline();
        self.indent_level -= 1;
        self.write_indent();
        print::print("}");
        self.needs_comma = true;
    }

    /// Begin a JSON array `[`.
    pub fn begin_array(&mut self) {
        self.write_separator();
        self.write_indent();
        print::print("[");
        self.write_newline();
        self.indent_level += 1;
        self.needs_comma = false;
    }

    /// End a JSON array `]`.
    pub fn end_array(&mut self) {
        self.write_newline();
        self.indent_level -= 1;
        self.write_indent();
        print::print("]");
        self.needs_comma = true;
    }

    /// Write an object key.
    pub fn key(&mut self, name: &str) {
        self.write_separator();
        self.write_indent();
        print::print("\"");
        print_escaped(name);
        print::print("\":");
        if self.pretty {
            print::print(" ");
        }
        self.needs_comma = false;
    }

    /// Write a string value.
    pub fn value_string(&mut self, value: &str) {
        print::print("\"");
        print_escaped(value);
        print::print("\"");
        self.needs_comma = true;
    }

    /// Write an unsigned integer value.
    pub fn value_u64(&mut self, value: u64) {
        let mut buf = itoa::Buffer::new();
        print::print(buf.format(value));
        self.needs_comma = true;
    }

    /// Write a signed integer value.
    pub fn value_i64(&mut self, value: i64) {
        let mut buf = itoa::Buffer::new();
        print::print(buf.format(value));
        self.needs_comma = true;
    }

    /// Write a boolean value.
    pub fn value_bool(&mut self, value: bool) {
        print::print(if value { "true" } else { "false" });
        self.needs_comma = true;
    }

    /// Write a null value.
    pub fn value_null(&mut self) {
        print::print("null");
        self.needs_comma = true;
    }

    /// Write a key-value pair with a string value.
    pub fn field_str(&mut self, key: &str, value: &str) {
        self.key(key);
        self.value_string(value);
    }

    /// Write a key-value pair with an optional string value.
    pub fn field_str_opt(&mut self, key: &str, value: Option<&str>) {
        if let Some(v) = value {
            self.field_str(key, v);
        }
    }

    /// Write a key-value pair with a u64 value.
    pub fn field_u64(&mut self, key: &str, value: u64) {
        self.key(key);
        self.value_u64(value);
    }

    /// Write a key-value pair with an optional u64 value.
    pub fn field_u64_opt(&mut self, key: &str, value: Option<u64>) {
        if let Some(v) = value {
            self.field_u64(key, v);
        }
    }

    /// Write a key-value pair with an i64 value.
    pub fn field_i64(&mut self, key: &str, value: i64) {
        self.key(key);
        self.value_i64(value);
    }

    /// Write a key-value pair with a boolean value.
    pub fn field_bool(&mut self, key: &str, value: bool) {
        self.key(key);
        self.value_bool(value);
    }

    /// Begin an object value for a key.
    pub fn field_object(&mut self, key: &str) {
        self.key(key);
        self.needs_comma = false;
        print::print("{");
        self.write_newline();
        self.indent_level += 1;
    }

    /// End an object that was started with field_object.
    pub fn end_field_object(&mut self) {
        self.write_newline();
        self.indent_level -= 1;
        self.write_indent();
        print::print("}");
        self.needs_comma = true;
    }

    /// Begin an array value for a key.
    pub fn field_array(&mut self, key: &str) {
        self.key(key);
        self.needs_comma = false;
        print::print("[");
        self.write_newline();
        self.indent_level += 1;
    }

    /// End an array that was started with field_array.
    pub fn end_field_array(&mut self) {
        self.write_newline();
        self.indent_level -= 1;
        self.write_indent();
        print::print("]");
        self.needs_comma = true;
    }

    /// Write an array element that's a string.
    pub fn array_string(&mut self, value: &str) {
        self.write_separator();
        self.write_indent();
        print::print("\"");
        print_escaped(value);
        print::print("\"");
        self.needs_comma = true;
    }

    /// Begin an array element that's an object.
    pub fn array_object_begin(&mut self) {
        self.write_separator();
        self.write_indent();
        print::print("{");
        self.write_newline();
        self.indent_level += 1;
        self.needs_comma = false;
    }

    /// End an array element that's an object.
    pub fn array_object_end(&mut self) {
        self.write_newline();
        self.indent_level -= 1;
        self.write_indent();
        print::print("}");
        self.needs_comma = true;
    }
}

/// Print a string with JSON escaping directly to stdout.
fn print_escaped(s: &str) {
    for c in s.chars() {
        if c.is_ascii() {
            let byte = c as u8;
            match ESCAPE_TABLE[byte as usize] {
                EscapeType::None => print::print_char(c),
                EscapeType::Backspace => print::print("\\b"),
                EscapeType::Tab => print::print("\\t"),
                EscapeType::Newline => print::print("\\n"),
                EscapeType::FormFeed => print::print("\\f"),
                EscapeType::CarriageReturn => print::print("\\r"),
                EscapeType::Quote => print::print("\\\""),
                EscapeType::Backslash => print::print("\\\\"),
                EscapeType::Unicode => {
                    print::print("\\u00");
                    print::print_char(byte.hex_hi());
                    print::print_char(byte.hex_lo());
                }
            }
        } else {
            print::print_char(c);
        }
    }
}

/// Helper to create the standard kv JSON envelope (streaming version).
pub fn begin_kv_output_streaming(pretty: bool, subcommand: &str) -> StreamingJsonWriter {
    let mut w = StreamingJsonWriter::new(pretty);
    w.begin_object();
    w.field_str("kv_version", env!("CARGO_PKG_VERSION"));
    w.field_str("subcommand", subcommand);
    w
}
