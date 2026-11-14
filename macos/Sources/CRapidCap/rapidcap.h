// C surface of crates/ffi. Every char* returned is JSON owned by Rust: free it
// with rc_free. Failures are {"error": "..."}.
#pragma once
#include <stdbool.h>
#include <stdint.h>

typedef struct Recording Recording;

void rc_free(char *text);
char *rc_load(void);
char *rc_save_settings(const char *json);
char *rc_screenshot(int32_t x, int32_t y, uint32_t width, uint32_t height,
                    const char *process_name);
Recording *rc_record_start(uint32_t kind, int32_t x, int32_t y, uint32_t width,
                           uint32_t height, char **error_out);
char *rc_record_pause(Recording *recording);
char *rc_record_resume(Recording *recording);
char *rc_record_stop(Recording *recording);
void rc_record_cancel(Recording *recording);
char *rc_recording_paths(bool gif);
char *rc_encode_gif(const char *source, const char *gif);
char *rc_microphones(void);
char *rc_copy_file(const char *path);
char *rc_mux_audio(const char *video, const char *audio, double audio_seconds);
