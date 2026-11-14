// Vendored from re_ui 0.42.0 (https://github.com/rerun-io/rerun), MIT OR Apache-2.0.
// Upstream decides the `hot_reload_design_tokens` cfg here with `re_build_tools`; the port keeps
// only the declaration (the hot-reload branches themselves are removed from the sources), so that
// rustc knows the cfg without the rerun build machinery.
fn main() {
    println!("cargo::rustc-check-cfg=cfg(hot_reload_design_tokens)");
}
