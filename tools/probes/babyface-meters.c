/* SPDX-License-Identifier: MIT
 * Copyright (c) 2026 Ismaïl Bahloul
 *
 * Read-only probe of the Babyface Pro FS's hardware meters: bulk IN 0x85 on
 * interface 1, which the audio driver leaves alone, so it runs alongside
 * snd-usb-babyface-pro. Prints the peak of each of the 40 slots in dBFS,
 * see tuxmix-core/src/babyface_meters.rs for what the slots are.
 *
 *   cc -O2 -o babyface-meters tools/probes/babyface-meters.c -lusb-1.0 -lm
 *   ./babyface-meters [packets]
 */
#include <libusb-1.0/libusb.h>
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

int main(int argc, char **argv)
{
	int count = argc > 1 ? atoi(argv[1]) : 100;
	libusb_context *ctx;
	libusb_device_handle *h;
	unsigned char b[1024];
	double peak[40];
	int n = 0, i, p;

	libusb_init(&ctx);
	h = libusb_open_device_with_vid_pid(ctx, 0x2a39, 0x3fc0);
	if (!h || libusb_claim_interface(h, 1)) {
		fprintf(stderr, "no Babyface Pro in proprietary mode\n");
		return 1;
	}
	for (i = 0; i < 40; i++)
		peak[i] = -200;
	for (p = 0; p < count; p++) {
		usleep(10000);
		if (libusb_bulk_transfer(h, 0x85, b, sizeof(b), &n, 500) || n != 480)
			continue;
		for (i = 0; i < 40; i++) {
			uint32_t v;
			double db;

			memcpy(&v, b + 320 + i * 4, 4);
			v &= ~0xfu;	/* the low four bits are flags */
			db = v ? 20 * log10((double)v / (double)(1u << 27)) : -200;
			if (db > peak[i])
				peak[i] = db;
		}
	}
	printf("inputs   :");
	for (i = 0; i < 12; i++)
		printf(" %6.1f", peak[i]);
	printf("\nplayback :");
	for (i = 12; i < 26; i++)
		printf(" %6.1f", peak[i]);
	printf("\noutputs  :");
	for (i = 26; i < 38; i++)
		printf(" %6.1f", peak[i]);
	printf("\nFX bus   : %6.1f %6.1f\n", peak[38], peak[39]);
	libusb_release_interface(h, 1);
	libusb_close(h);
	libusb_exit(ctx);
	return 0;
}
