//! Native libvips link discovery for platforms with non-system prefixes.

use std::env;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_PATH");
    if env::var_os("CARGO_FEATURE_NATIVE_VIPS").is_some() {
        pkg_config::Config::new()
            // Older distro packages may compile the stable calls used here; production
            // capability approval still requires the 8.18 fixture smoke.
            .atleast_version("8.15")
            .probe("vips")?;
    }
    Ok(())
}
