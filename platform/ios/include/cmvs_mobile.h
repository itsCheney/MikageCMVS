#ifndef CMVS_MOBILE_H
#define CMVS_MOBILE_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* The C interface of the CMVS runtime for app hosts (crates/cmvs-mobile).
 *
 * A session runs one game: open it, give it a view to draw into, then call
 * cmvs_step once per display frame (CADisplayLink) and pass the input on.
 * The engine draws with Metal (wgpu) into the view's CAMetalLayer, fitted
 * with black bars. Sound is pulled by the host: render cmvs_audio_render
 * from its audio callback.
 *
 * Every call except the cmvs_audio_* ones belongs on the thread that opened
 * the session (the main thread). Strings are UTF-8 and borrowed for the
 * duration of the call. */

#define CMVS_ABI_VERSION 2u

typedef struct CmvsSession CmvsSession;
typedef struct CmvsAudio CmvsAudio;

typedef struct {
    uint32_t struct_size;   /* sizeof(CmvsConfig) */
    const char *game_dir;   /* the game folder (with data/pack/start.ps3) */
    const char *save_dir;   /* saves and settings, or NULL for <game_dir>/savedata */
    uint32_t encoding;      /* one of CMVS_ENCODING_* */
    int64_t utc_offset_ms;  /* local time offset, for the dates on save slots */
} CmvsConfig;

enum {
    CMVS_ENCODING_AUTOMATIC = 0, /* GBK when the game has a Chinese patch (Pk/
                                    or Longinus.dat), else Shift-JIS */
    CMVS_ENCODING_SHIFT_JIS = 1,
    CMVS_ENCODING_GBK = 2,
};

enum {
    CMVS_RUNNING = 0,
    CMVS_ENDED = 1,   /* the game quit */
    CMVS_FAILED = 2,  /* see cmvs_last_error: an unsupported command, a broken
                         file... The last frame stays on screen. */
};

enum {
    CMVS_POINTER_MOVE = 0,
    CMVS_POINTER_LEFT_DOWN = 1,
    CMVS_POINTER_LEFT_UP = 2,
    CMVS_POINTER_RIGHT_DOWN = 3,
    CMVS_POINTER_RIGHT_UP = 4,
    CMVS_POINTER_MIDDLE_DOWN = 5,
    CMVS_POINTER_MIDDLE_UP = 6,
};

uint32_t cmvs_abi_version(void);

/* The last error on this thread, or "" when none. Valid until the next call
 * on this thread. */
const char *cmvs_last_error(void);

/* Engine log records: level 1 error, 2 warn, 3 info, 4 debug. Strings are
 * valid during the call; any thread. NULL stops them. Records include the
 * engine profile chosen for the game and every command that runs on presumed
 * semantics, which is what a compatibility report needs. */
typedef void (*cmvs_log_callback)(int32_t level, const char *target, const char *message, void *context);
void cmvs_set_log_callback(cmvs_log_callback callback, void *context, int32_t max_level);

/* Fonts for the sessions opened after the call: a TTF, OTF or TTC file and
 * the face in it, as a family name ("Hiragino Sans") or a collection index
 * ("2"); NULL or "" for the first face. The first font registered draws the
 * game's default face (MS Gothic on Windows); the folders of all registered
 * fonts are searched for faces the game names. Returns 1, or 0 when the file
 * is missing; a face that is not in the file fails cmvs_open. */
int32_t cmvs_register_font(const char *path, const char *face);
void cmvs_clear_fonts(void);

/* Opens and boots a game. NULL on failure (see cmvs_last_error). Games that
 * the engine has not recorded (updated releases, patched copies) still open:
 * the profile is taken from the engine executable in the folder, or the
 * 2016 baseline, and the log says so. A translation patch that hooks the
 * engine with a DLL is read in place of the game's own files: Pk/*.pk
 * (scripts, PNG images) with Pk/*.cmv (movies), or Longinus.dat (scripts,
 * pixels) with data/pack/start.CHS. */
CmvsSession *cmvs_open(const CmvsConfig *config);

/* The game's screen size in pixels (1280 x 720). */
void cmvs_screen_size(const CmvsSession *session, uint32_t *width, uint32_t *height);

/* Draw into ui_view, a UIView whose layer is a CAMetalLayer, of
 * width x height drawable pixels. Call again after the size changed. The
 * view must outlive the session or cmvs_detach_surface. Returns 0, or -1 on
 * failure (see cmvs_last_error). */
int32_t cmvs_set_surface(CmvsSession *session, void *ui_view, uint32_t width_px, uint32_t height_px);
void cmvs_detach_surface(CmvsSession *session);

/* Advances the clock by dt_ms and runs the game for up to about 12 ms;
 * draws when the game finished a frame. Returns CMVS_RUNNING, CMVS_ENDED or
 * CMVS_FAILED. */
int32_t cmvs_step(CmvsSession *session, uint32_t dt_ms);

/* The pointer, in drawable pixels of the surface. One of CMVS_POINTER_*. */
void cmvs_pointer(CmvsSession *session, int32_t phase, float x, float y);
/* Mouse wheel: positive notches up, negative down. */
void cmvs_wheel(CmvsSession *session, int32_t notches);
/* Windows virtual-key code: 0x0D Enter, 0x1B Escape, 0x11 Ctrl, 0x20 Space,
 * 0x70.. F1..; key bindings come from the game's key.cfg. */
void cmvs_key(CmvsSession *session, int32_t vk, int32_t down);
/* The app left (0) or regained (1) the foreground: held keys are released
 * and the game sees its window lose focus. */
void cmvs_set_focused(CmvsSession *session, int32_t focused);

/* Sound: an output fed from the session's mixer by a thread of its own, once
 * per session. cmvs_audio_render never blocks or allocates, so it can run on
 * a real-time audio thread; call it from one thread at a time. What is
 * rendered runs about 93 ms ahead of the game's sound positions. Release it
 * after cmvs_close. Movies follow their soundtrack's position, so they
 * advance only while the host renders this output. */
CmvsAudio *cmvs_audio(CmvsSession *session);
uint32_t cmvs_audio_sample_rate(void);
/* Writes frames interleaved stereo float samples (2 * frames floats). */
void cmvs_audio_render(CmvsAudio *audio, float *out, uint32_t frames);
void cmvs_audio_release(CmvsAudio *audio);

/* Saves the game's settings and ends the session. */
void cmvs_close(CmvsSession *session);

#ifdef __cplusplus
}
#endif

#endif
