# Build and install grokbot-tui from this checkout (no `cargo install`, no
# downloaded binaries). Building needs a Rust toolchain. All work is done by
# install-local.sh; see it for the details.
#
#   make                      build (release)
#   make install              build, then install under PREFIX
#   make uninstall
#   make vendor               cargo vendor into ./vendor for offline builds
#   Variables: PREFIX (default ~/.local, /usr/local as root), DESTDIR,
#              FFMPEG=1 (-ffmpeg variant, FFmpeg linked statically),
#              OFFLINE=1 (build with --offline, using ./vendor if present),
#              NOBUILD=1 (install an existing target/release build)
SH := ./install-local.sh
export PREFIX DESTDIR FFMPEG OFFLINE NOBUILD VENDOR

.PHONY: all build install uninstall vendor clean
all: build
build:
	$(SH) --build-only
install:
	$(SH)
uninstall:
	$(SH) --uninstall
vendor:
	cargo vendor --locked vendor
clean:
	cargo clean
