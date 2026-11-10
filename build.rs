//! Build script for the project

/// Build the image resource (SVG) so the application can reference the image internally
fn main() {
    glib_build_tools::compile_resources(&["data"], "data/lumina.gresource.xml", "lumina.gresource");
}
