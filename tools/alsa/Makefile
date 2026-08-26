# ALSA plugin for the TuxMix Babyface Pro FS proprietary driver.
#
# Build:       make
# Install:     make install   (needs root: copies the .so + the ALSA conf)
# Uninstall:   make uninstall
#
# The Rust side must be built first (release):  cargo build -p tuxmix-sys --release

CC      ?= gcc
TARGET   = libasound_module_pcm_tuxmix.so
RELEASE  = ../../target/release
CFLAGS  += -O2 -Wall -Wextra -fPIC -DPIC -I../../tuxmix-sys/include $(shell pkg-config --cflags alsa)
LIBS    += -shared -lasound -L$(RELEASE) -ltuxmix_sys
# For local testing: the plugin finds libtuxmix_sys.so in the cargo
# target dir. For a system install, copy libtuxmix_sys.so to /usr/lib
# and drop the -rpath (see install).
RPATH   := -Wl,-rpath,$(abspath $(RELEASE))

all: $(TARGET)

$(TARGET): pcm_tuxmix.c ../../tuxmix-sys/include/tuxmix_sys.h
	$(CC) $(CFLAGS) -o $@ pcm_tuxmix.c $(LIBS) $(RPATH)

install: $(TARGET)
	install -D -m 644 $(TARGET) /usr/lib/alsa-lib/$(TARGET)
	install -D -m 644 ../../tuxmix-sys/include/tuxmix_sys.h /usr/include/tuxmix_sys.h
	install -D -m 644 $(RELEASE)/libtuxmix_sys.so /usr/lib/libtuxmix_sys.so
	install -D -m 644 50-tuxmix.conf /etc/alsa/conf.d/50-tuxmix.conf
	ldconfig

uninstall:
	rm -f /usr/lib/alsa-lib/$(TARGET)
	rm -f /usr/include/tuxmix_sys.h
	rm -f /usr/lib/libtuxmix_sys.so
	rm -f /etc/alsa/conf.d/50-tuxmix.conf

clean:
	rm -f $(TARGET)

.PHONY: all install uninstall clean
