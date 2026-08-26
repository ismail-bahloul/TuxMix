/* tuxmix_sys.h — C ABI of the TuxMix Babyface Pro FS audio driver
 * (tuxmix-sys, a Rust cdylib). All audio buffers are interleaved
 * S24_LE (3 bytes/sample):
 *   - playback: 4 channels/frame (device ch0-3 = PB1+PB2)
 *   - capture:  4 channels/frame (AN1-4)
 * Every function is thread-safe (the handle is internally mutexed).
 */
#ifndef TUXMIX_SYS_H
#define TUXMIX_SYS_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define TUXMIX_PLAYBACK_CHANNELS 4
#define TUXMIX_CAPTURE_CHANNELS 4
#define TUXMIX_S24_LE_BYTES 3

/* Returns a handle or NULL on failure. */
void *tuxmix_audio_open(void);
/* Start/stop the streaming session. 0 = ok. */
int tuxmix_audio_start(void *h);
int tuxmix_audio_stop(void *h);
void tuxmix_audio_close(void *h);
/* Sample rate in Hz. set_rate returns 0 = ok. */
int tuxmix_audio_set_rate(void *h, uint32_t rate);
uint32_t tuxmix_audio_rate(void *h);
/* Frames of interleaved S24_LE pushed / read (channels = 2 or 4). */
size_t tuxmix_audio_write_playback(void *h, const void *buf, size_t frames, size_t channels);
size_t tuxmix_audio_read_capture(void *h, void *buf, size_t frames, size_t channels);
/* Capture wakeup fd (readable when capture frames are queued). */
int tuxmix_audio_capture_fd(void *h);
/* Frames queued (snd_pcm_delay) / monotonic hw positions. */
size_t tuxmix_audio_capture_queued(void *h);
size_t tuxmix_audio_playback_queued(void *h);
size_t tuxmix_audio_playback_capacity(void *h);
uint64_t tuxmix_audio_playback_pushed(void *h);
uint64_t tuxmix_audio_capture_pushed(void *h);

#ifdef __cplusplus
}
#endif

#endif /* TUXMIX_SYS_H */
