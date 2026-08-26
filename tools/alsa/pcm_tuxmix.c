/*
 * pcm_tuxmix.c — ALSA PCM plugin for the RME Babyface Pro FS in
 * PROPRIETARY mode (the TotalMix protocol, reverse-engineered by the
 * TuxMix project). Backed by the Rust driver through `tuxmix-sys`.
 *
 * The device streams on interface 5 (interrupt ep 0x01 OUT / 0x82 IN),
 * 14×32-bit frames (24-bit audio in bytes 1-3). This plugin exposes:
 *   - playback: 4 channels (PB1 + PB2 into the TotalMix mixer)
 *   - capture:  4 channels (AN1-4)
 * at the device's native rates (32-192 kHz), format S24_LE.
 *
 * Build:  make        (outputs libasound_module_pcm_tuxmix.so)
 * Install: make install (into /usr/lib/alsa-lib)
 * Config:  pcm.tuxmix { type tuxmix }  (see alsa.conf.d/50-tuxmix.conf)
 *
 * The mixer itself stays with the TuxMix GUI/TUI (this plugin only
 * moves audio).
 */

#include <alsa/asoundlib.h>
#include <alsa/pcm_external.h> /* defines the ioplug plugin SDK */
#include <poll.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#include "tuxmix_sys.h"

/* The sample rates the device supports (protocol::rate_to_alt). */
static const unsigned int tuxmix_rates[] = {
	32000, 44100, 48000, 64000, 88200, 96000, 128000, 176400, 192000,
};

typedef struct {
	snd_pcm_ioplug_t io;
	void *h;      /* tuxmix-sys handle */
	int wake_fd;  /* capture wakeup (poll descriptor) */
} tuxmix_t;

static int tuxmix_start(snd_pcm_ioplug_t *io)
{
	tuxmix_t *t = io->private_data;
	return tuxmix_audio_start(t->h);
}

static int tuxmix_stop(snd_pcm_ioplug_t *io)
{
	tuxmix_t *t = io->private_data;
	return tuxmix_audio_stop(t->h);
}

static snd_pcm_sframes_t tuxmix_pointer(snd_pcm_ioplug_t *io)
{
	tuxmix_t *t = io->private_data;
	uint64_t pos;
	if (io->stream == SND_PCM_STREAM_PLAYBACK) {
		pos = tuxmix_audio_playback_pushed(t->h);
		pos -= tuxmix_audio_playback_queued(t->h);
	} else {
		pos = tuxmix_audio_capture_pushed(t->h);
	}
	return (snd_pcm_sframes_t)(pos % io->buffer_size);
}

static snd_pcm_sframes_t tuxmix_transfer(snd_pcm_ioplug_t *io,
					 const snd_pcm_channel_area_t *areas,
					 snd_pcm_uframes_t offset,
					 snd_pcm_uframes_t size)
{
	tuxmix_t *t = io->private_data;
	const unsigned int stride = io->channels * TUXMIX_S24_LE_BYTES;
	if (io->stream == SND_PCM_STREAM_PLAYBACK) {
		const char *src = (const char *)areas[0].addr + offset * stride;
		return (snd_pcm_sframes_t)tuxmix_audio_write_playback(t->h, src,
									size, io->channels);
	} else {
		char *dst = (char *)areas[0].addr + offset * stride;
		snd_pcm_uframes_t got = 0;
		while (got < size) {
			got += tuxmix_audio_read_capture(t->h,
					dst + got * stride, size - got, io->channels);
			if (got >= size || io->nonblock)
				break;
			/* Wait for the pump thread to fill more frames. */
			struct pollfd p = { .fd = t->wake_fd, .events = POLLIN };
			if (poll(&p, 1, 200) <= 0)
				break;
		}
		return (snd_pcm_sframes_t)got;
	}
}

static int tuxmix_hw_params(snd_pcm_ioplug_t *io, snd_pcm_hw_params_t *params)
{
	tuxmix_t *t = io->private_data;
	(void)params;
	return tuxmix_audio_set_rate(t->h, io->rate);
}

static int tuxmix_delay(snd_pcm_ioplug_t *io, snd_pcm_sframes_t *delayp)
{
	tuxmix_t *t = io->private_data;
	*delayp = (snd_pcm_sframes_t)(io->stream == SND_PCM_STREAM_PLAYBACK
				     ? tuxmix_audio_playback_queued(t->h) : 0);
	return 0;
}

static int tuxmix_poll_count(snd_pcm_ioplug_t *io)
{
	(void)io;
	return 1;
}

static int tuxmix_poll_fds(snd_pcm_ioplug_t *io, struct pollfd *pfd,
			   unsigned int space)
{
	tuxmix_t *t = io->private_data;
	if (space < 1)
		return -ENOSPC;
	pfd[0].fd = t->wake_fd;
	pfd[0].events = POLLIN;
	return 1;
}

static int tuxmix_poll_revents(snd_pcm_ioplug_t *io, struct pollfd *pfd,
			       unsigned int nfds, unsigned short *revents)
{
	tuxmix_t *t = io->private_data;
	(void)pfd;
	(void)nfds;
	if (io->stream == SND_PCM_STREAM_PLAYBACK) {
		/* The ring has space -> writable. When full, wait for the
		 * pump thread to free space (it signals the wake fd). */
		if (tuxmix_audio_playback_queued(t->h) <
		    tuxmix_audio_playback_capacity(t->h))
			*revents = POLLOUT;
		else
			*revents = 0;
		return 0;
	}
	*revents = 0;
	if (nfds >= 1 && (pfd[0].revents & POLLIN))
		*revents |= POLLIN;
	return 0;
}

static int tuxmix_close(snd_pcm_ioplug_t *io)
{
	tuxmix_t *t = io->private_data;
	tuxmix_audio_stop(t->h);
	tuxmix_audio_close(t->h);
	return 0;
}

static void tuxmix_dump(snd_pcm_ioplug_t *io, snd_output_t *out)
{
	tuxmix_t *t = io->private_data;
	(void)t;
	snd_output_printf(out, "TuxMix Babyface Pro FS (%s, S24_LE, %u ch)\n",
			  io->stream == SND_PCM_STREAM_PLAYBACK ?
			  "playback" : "capture", io->channels);
}

static const snd_pcm_ioplug_callback_t tuxmix_callback = {
	.start = tuxmix_start,
	.stop = tuxmix_stop,
	.pointer = tuxmix_pointer,
	.transfer = tuxmix_transfer,
	.hw_params = tuxmix_hw_params,
	.close = tuxmix_close,
	.delay = tuxmix_delay,
	.poll_descriptors_count = tuxmix_poll_count,
	.poll_descriptors = tuxmix_poll_fds,
	.poll_revents = tuxmix_poll_revents,
	.dump = tuxmix_dump,
};

SND_PCM_PLUGIN_DEFINE_FUNC(tuxmix)
{
	tuxmix_t *t;
	int err;

	t = calloc(1, sizeof(*t));
	if (!t)
		return -ENOMEM;

	t->h = tuxmix_audio_open();
	if (!t->h) {
		free(t);
		return -ENODEV;
	}
	t->wake_fd = tuxmix_audio_capture_fd(t->h);

	t->io.version = SND_PCM_IOPLUG_VERSION;
	t->io.name = "TuxMix Babyface Pro FS";
	t->io.flags = SND_PCM_IOPLUG_FLAG_LISTED;
	t->io.callback = &tuxmix_callback;
	t->io.private_data = t;

	err = snd_pcm_ioplug_create(&t->io, name, stream, mode);
	if (err < 0)
		goto error;

	err = snd_pcm_ioplug_set_param_list(&t->io, SND_PCM_IOPLUG_HW_ACCESS,
					    1,
					    (const unsigned int[]) {
						SND_PCM_ACCESS_RW_INTERLEAVED });
	if (err < 0)
		goto error;
	/* S24_3LE = 24-bit in 3 bytes — exactly the device's frame sample. */
	err = snd_pcm_ioplug_set_param_list(&t->io, SND_PCM_IOPLUG_HW_FORMAT,
					    1,
					    (const unsigned int[]) {
						SND_PCM_FORMAT_S24_3LE });
	if (err < 0)
		goto error;
	/* 2 (PB1, the PipeWire stereo default) or 4 (PB1+PB2) channels. */
	err = snd_pcm_ioplug_set_param_minmax(&t->io,
					      SND_PCM_IOPLUG_HW_CHANNELS,
					      2, 4);
	if (err < 0)
		goto error;
	err = snd_pcm_ioplug_set_param_list(&t->io, SND_PCM_IOPLUG_HW_RATE,
					    (unsigned int)
					    (sizeof(tuxmix_rates) /
					     sizeof(tuxmix_rates[0])),
					    tuxmix_rates);
	if (err < 0)
		goto error;
	err = snd_pcm_ioplug_set_param_minmax(&t->io,
					      SND_PCM_IOPLUG_HW_PERIOD_BYTES,
					      1024, 1024 * 1024);
	if (err < 0)
		goto error;
	err = snd_pcm_ioplug_set_param_minmax(&t->io,
					      SND_PCM_IOPLUG_HW_BUFFER_BYTES,
					      4096, 4 * 1024 * 1024);
	if (err < 0)
		goto error;

	*pcmp = t->io.pcm;
	return 0;

error:
	snd_pcm_ioplug_delete(&t->io);
	tuxmix_audio_close(t->h);
	free(t);
	return err;
}

SND_PCM_PLUGIN_SYMBOL(tuxmix);
