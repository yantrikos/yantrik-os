// A test renderer for zlabwc_window_switcher_renderer_v1: logs every event to
// $SWITCHER_PROBE_LOG (default /tmp/switcher-probe.log), acks each session after
// its first windows_done unless SWITCHER_PROBE_NO_ACK is set, and on a 'z' key
// asks to select the last window of the list (to test the select request).
// While /tmp/switcher-probe-capture exists when it starts, it binds version 2 and
// asks for a picture of the last window after each windows_done (to test capture).
#define _POSIX_C_SOURCE 200809L
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>
#include <wayland-client.h>
#include "labwc-window-switcher-renderer-unstable-v1-client-protocol.h"

static FILE *out;
static struct zlabwc_window_switcher_renderer_v1 *renderer;
static int no_ack;
static uint32_t acked_session, last_id;

static void on_start(void *d, struct zlabwc_window_switcher_renderer_v1 *r, uint32_t s)
{ fprintf(out, "session_start %u\n", s); fflush(out); }
static void on_window(void *d, struct zlabwc_window_switcher_renderer_v1 *r, uint32_t s,
		uint32_t id, const char *app_id, const char *title, const char *ws, uint32_t flags, uint32_t idle)
{ fprintf(out, "window %u id=%u app_id=%s title=%s ws=%s flags=%u idle_ms=%u\n", s, id, app_id, title, ws, flags, idle);
  last_id = id; fflush(out); }
static void on_done(void *d, struct zlabwc_window_switcher_renderer_v1 *r, uint32_t s)
{ fprintf(out, "windows_done %u\n", s);
  if (!no_ack && access("/tmp/switcher-probe-noack", F_OK) != 0 && acked_session != s) { zlabwc_window_switcher_renderer_v1_ack(r, s); acked_session = s; fprintf(out, "-> ack %u\n", s); }
  if (zlabwc_window_switcher_renderer_v1_get_version(r) >= 2) { zlabwc_window_switcher_renderer_v1_capture(r, s, last_id, 240, 240); fprintf(out, "-> capture %u id=%u\n", s, last_id); }
  fflush(out); }
static void on_selected(void *d, struct zlabwc_window_switcher_renderer_v1 *r, uint32_t s, uint32_t id)
{ fprintf(out, "selected %u id=%u\n", s, id); fflush(out); }
static void on_key(void *d, struct zlabwc_window_switcher_renderer_v1 *r, uint32_t s, uint32_t sym, const char *utf8)
{ fprintf(out, "key %u sym=0x%x utf8=%s\n", s, sym, utf8);
  if (!strcmp(utf8, "z")) { zlabwc_window_switcher_renderer_v1_select(r, s, last_id); fprintf(out, "-> select %u id=%u\n", s, last_id); }
  fflush(out); }
static void on_ended(void *d, struct zlabwc_window_switcher_renderer_v1 *r, uint32_t s, uint32_t reason)
{ fprintf(out, "ended %u reason=%s\n", s, reason ? "committed" : "cancelled"); fflush(out); }
static void on_thumbnail(void *d, struct zlabwc_window_switcher_renderer_v1 *r, uint32_t s, uint32_t id,
		int32_t fd, uint32_t w, uint32_t h, uint32_t stride)
{ size_t size = (size_t)stride * h;
  const unsigned char *px = mmap(NULL, size, PROT_READ, MAP_PRIVATE, fd, 0);
  unsigned long long sum = 0;
  int mapped = px != MAP_FAILED;
  if (mapped) { for (size_t i = 0; i < size; i++) sum += px[i]; munmap((void *)px, size); }
  fprintf(out, "thumbnail %u id=%u %ux%u stride=%u mapped=%s byte_mean=%.1f\n", s, id, w, h, stride,
    mapped ? "yes" : "no", mapped && size ? (double)sum / size : 0.0);
  close(fd); fflush(out); }

static const struct zlabwc_window_switcher_renderer_v1_listener listener = {
	.session_start = on_start, .window = on_window, .windows_done = on_done,
	.selected = on_selected, .key = on_key, .ended = on_ended, .thumbnail = on_thumbnail,
};

static void global(void *d, struct wl_registry *reg, uint32_t name, const char *iface, uint32_t ver)
{
	if (!strcmp(iface, zlabwc_window_switcher_renderer_v1_interface.name)) {
		int v2 = ver >= 2 && access("/tmp/switcher-probe-capture", F_OK) == 0;
		renderer = wl_registry_bind(reg, name, &zlabwc_window_switcher_renderer_v1_interface, v2 ? 2 : 1);
		zlabwc_window_switcher_renderer_v1_add_listener(renderer, &listener, NULL);
	}
}
static void global_remove(void *d, struct wl_registry *reg, uint32_t name) {}
static const struct wl_registry_listener reg_listener = { global, global_remove };

int main(void)
{
	const char *path = getenv("SWITCHER_PROBE_LOG");
	out = fopen(path ? path : "/tmp/switcher-probe.log", "a");
	if (!out) return 1;
	no_ack = getenv("SWITCHER_PROBE_NO_ACK") != NULL;
	struct wl_display *dpy = wl_display_connect(NULL);
	if (!dpy) { fprintf(out, "connect failed\n"); return 1; }
	struct wl_registry *reg = wl_display_get_registry(dpy);
	wl_registry_add_listener(reg, &reg_listener, NULL);
	wl_display_roundtrip(dpy);
	fprintf(out, "started: renderer global %s, no_ack=%d\n", renderer ? "bound" : "NOT OFFERED", no_ack);
	fflush(out);
	while (wl_display_dispatch(dpy) != -1) {}
	fprintf(out, "display gone\n");
	return 0;
}
