fn main() {
    glib_build_tools::compile_resources(
        &["data/icons/hicolor"],
        "data/io.github.h0ek.DeskEntry.gresource.xml",
        "deskentry.gresource",
    );
}
