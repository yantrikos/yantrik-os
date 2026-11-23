// A test client for zyantrik_window_locator_v1. labwc launches it from
// <yantrik locator="/path/locator-probe">; it logs every answer to $LOCATOR_PROBE_LOG
// (default /tmp/locator-probe.log) and takes commands, one per line, from
// /tmp/locator-probe.cmd, or $LOCATOR_PROBE_CMD (read and removed every 100 ms):
//
//   at X Y            which window is at layout X, Y
//   focused           which window has focus
//   capture ID W H    a picture of window ID (or "last": the last id answered), at most W x H;
//                     written to /tmp/locator-probe-<id>-<n>.ppm, and the seal checked
//   flood N X Y       N at requests in one go (the 120 a second limit)
//   wait MS           wait, answering events
//   overlay on|off    a full-screen top-layer surface of its own, as the drag overlay will be
//   passthru on|off   the same, but letting clicks through (an empty input region)
//   move X Y          move the pointer to layout X, Y (virtual pointer: the compositor's
//                     own protocol, which any unsandboxed client may use)
//   press / release   hold / let go of the left button (virtual pointer)
//
// The locator answers `at` only while a button is held at the cursor, and `focused` only
// after an armed event (the YantrikLocate action); both are logged.
//
// Started by hand (not by labwc) it logs that the global was not offered.
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <time.h>
#include <unistd.h>
#include <wayland-client.h>
#include "yantrik-window-locator-unstable-v1-client-protocol.h"
#include "wlr-layer-shell-unstable-v1-client-protocol.h"
#include "wlr-virtual-pointer-unstable-v1-client-protocol.h"

static FILE *out;
static struct wl_display *dpy;
static struct zyantrik_window_locator_v1 *locator;
static struct wl_compositor *compositor;
static struct wl_shm *shm;
static struct zwlr_layer_shell_v1 *layer_shell;
static struct wl_surface *overlay_surface;
static struct zwlr_layer_surface_v1 *overlay;
static int overlay_passthru;
static struct zwlr_virtual_pointer_manager_v1 *vp_manager;
static struct zwlr_virtual_pointer_v1 *vp;
static uint32_t serial_next = 1, last_id;
static int pictures;
static const char *cmd_path = "/tmp/locator-probe.cmd";

static long long
ms(void)
{
	struct timespec ts;
	clock_gettime(CLOCK_MONOTONIC, &ts);
	return ts.tv_sec * 1000LL + ts.tv_nsec / 1000000;
}

#define LOG(...) do { fprintf(out, "%lld ", ms()); fprintf(out, __VA_ARGS__); fputc('\n', out); fflush(out); } while (0)

static void
on_window(void *d, struct zyantrik_window_locator_v1 *l, uint32_t serial, uint32_t id,
		const char *app_id, const char *title, int32_t x, int32_t y, uint32_t w, uint32_t h,
		uint32_t flags)
{
	LOG("window serial=%u id=%u app_id=%s title=%s box=%d,%d %ux%u flags=%u", serial, id, app_id,
		title, x, y, w, h, flags);
	last_id = id;
}

static const char *none_reasons[] = { "nothing", "locked", "throttled", "no_gesture" };
static const char *capture_failures[] = { "not_located", "gone", "locked", "throttled", "failed", "private" };

static void
on_none(void *d, struct zyantrik_window_locator_v1 *l, uint32_t serial, uint32_t reason)
{
	LOG("none serial=%u reason=%s", serial, reason < 4 ? none_reasons[reason] : "?");
}

static void
on_picture(void *d, struct zyantrik_window_locator_v1 *l, uint32_t id, int32_t fd, uint32_t w,
		uint32_t h, uint32_t stride)
{
	size_t size = (size_t)stride * h;
	int seals = fcntl(fd, F_GET_SEALS);
	void *rw = mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
	int writable = rw != MAP_FAILED;
	if (writable) {
		munmap(rw, size);
	}
	const uint8_t *px = mmap(NULL, size, PROT_READ, MAP_PRIVATE, fd, 0);
	if (px == MAP_FAILED) {
		LOG("picture id=%u %ux%u stride=%u: mmap failed: %s", id, w, h, stride, strerror(errno));
		close(fd);
		return;
	}
	char path[128];
	snprintf(path, sizeof(path), "/tmp/locator-probe-%u-%d.ppm", id, ++pictures);
	FILE *f = fopen(path, "wb");
	unsigned long long sum = 0;
	if (f) {
		fprintf(f, "P6\n%u %u\n255\n", w, h);
		for (uint32_t y = 0; y < h; y++) {
			for (uint32_t x = 0; x < w; x++) {
				const uint8_t *p = px + (size_t)y * stride + 4 * x; /* B G R A */
				fputc(p[2], f);
				fputc(p[1], f);
				fputc(p[0], f);
				sum += p[0] + p[1] + p[2];
			}
		}
		fclose(f);
	}
	LOG("picture id=%u %ux%u stride=%u seals=0x%x writable_shared_map=%s mean=%.1f file=%s", id,
		w, h, stride, seals, writable ? "YES" : "no", w && h ? sum / (3.0 * w * h) : 0.0, path);
	munmap((void *)px, size);
	close(fd);
}

static void
on_capture_failed(void *d, struct zyantrik_window_locator_v1 *l, uint32_t id, uint32_t reason)
{
	LOG("capture_failed id=%u reason=%s", id, reason < 6 ? capture_failures[reason] : "?");
}

static void
on_armed(void *d, struct zyantrik_window_locator_v1 *l)
{
	LOG("armed");
}

static const struct zyantrik_window_locator_v1_listener locator_listener = {
	.window = on_window, .none = on_none, .picture = on_picture,
	.capture_failed = on_capture_failed, .armed = on_armed,
};

static struct wl_buffer *
tint_buffer(int w, int h)
{
	int stride = w * 4;
	size_t size = (size_t)stride * h;
	int fd = memfd_create("locator-probe-overlay", MFD_CLOEXEC);
	if (fd < 0 || ftruncate(fd, (off_t)size) < 0) {
		return NULL;
	}
	uint32_t *p = mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
	for (size_t i = 0; i < size / 4; i++) {
		p[i] = 0x30102040; /* faint blue, premultiplied, so a screenshot shows it is up */
	}
	munmap(p, size);
	struct wl_shm_pool *pool = wl_shm_create_pool(shm, fd, (int32_t)size);
	struct wl_buffer *b = wl_shm_pool_create_buffer(pool, 0, w, h, stride, WL_SHM_FORMAT_ARGB8888);
	wl_shm_pool_destroy(pool);
	close(fd);
	return b;
}

static void
on_layer_configure(void *d, struct zwlr_layer_surface_v1 *s, uint32_t serial, uint32_t w, uint32_t h)
{
	zwlr_layer_surface_v1_ack_configure(s, serial);
	struct wl_buffer *b = tint_buffer((int)w, (int)h);
	if (overlay_passthru) {
		struct wl_region *empty = wl_compositor_create_region(compositor);
		wl_surface_set_input_region(overlay_surface, empty);
		wl_region_destroy(empty);
	}
	wl_surface_attach(overlay_surface, b, 0, 0);
	wl_surface_damage(overlay_surface, 0, 0, (int)w, (int)h);
	wl_surface_commit(overlay_surface);
	LOG("overlay up %ux%u", w, h);
}

static void
on_layer_closed(void *d, struct zwlr_layer_surface_v1 *s)
{
	LOG("overlay closed by compositor");
}

static const struct zwlr_layer_surface_v1_listener layer_listener = {
	.configure = on_layer_configure, .closed = on_layer_closed,
};

static void
overlay_set(int on)
{
	if (on && !overlay && compositor && shm && layer_shell) {
		overlay_surface = wl_compositor_create_surface(compositor);
		overlay = zwlr_layer_shell_v1_get_layer_surface(layer_shell, overlay_surface, NULL,
			ZWLR_LAYER_SHELL_V1_LAYER_TOP, "locator-probe-overlay");
		zwlr_layer_surface_v1_add_listener(overlay, &layer_listener, NULL);
		zwlr_layer_surface_v1_set_anchor(overlay, 15);
		zwlr_layer_surface_v1_set_exclusive_zone(overlay, -1);
		wl_surface_commit(overlay_surface);
	} else if (!on && overlay) {
		zwlr_layer_surface_v1_destroy(overlay);
		wl_surface_destroy(overlay_surface);
		overlay = NULL;
		overlay_surface = NULL;
		LOG("overlay down");
	}
}

static void
global(void *d, struct wl_registry *reg, uint32_t name, const char *iface, uint32_t ver)
{
	if (!strcmp(iface, zyantrik_window_locator_v1_interface.name)) {
		locator = wl_registry_bind(reg, name, &zyantrik_window_locator_v1_interface, 1);
		zyantrik_window_locator_v1_add_listener(locator, &locator_listener, NULL);
	} else if (!strcmp(iface, wl_compositor_interface.name)) {
		compositor = wl_registry_bind(reg, name, &wl_compositor_interface, 4);
	} else if (!strcmp(iface, wl_shm_interface.name)) {
		shm = wl_registry_bind(reg, name, &wl_shm_interface, 1);
	} else if (!strcmp(iface, zwlr_virtual_pointer_manager_v1_interface.name)) {
		vp_manager = wl_registry_bind(reg, name, &zwlr_virtual_pointer_manager_v1_interface, 1);
	} else if (!strcmp(iface, zwlr_layer_shell_v1_interface.name)) {
		layer_shell = wl_registry_bind(reg, name, &zwlr_layer_shell_v1_interface, 1);
	}
}

static void global_remove(void *d, struct wl_registry *reg, uint32_t name) {}
static const struct wl_registry_listener reg_listener = { global, global_remove };

static void
run_line(char *line)
{
	char cmd[32], a[32];
	int x = 0, y = 0, n = 0;
	unsigned w = 0, h = 0;
	if (sscanf(line, "%31s", cmd) != 1) {
		return;
	}
	LOG("> %s", line);
	if (!strcmp(cmd, "move") || !strcmp(cmd, "press") || !strcmp(cmd, "release")) {
		if (!vp && vp_manager) {
			vp = zwlr_virtual_pointer_manager_v1_create_virtual_pointer(vp_manager, NULL);
		}
		if (!vp) {
			LOG("no virtual pointer");
			return;
		}
		uint32_t t = (uint32_t)ms();
		if (!strcmp(cmd, "move") && sscanf(line, "%*s %d %d", &x, &y) == 2) {
			/* Extent of the nested output (1280x720): absolute = layout here */
			zwlr_virtual_pointer_v1_motion_absolute(vp, t, (uint32_t)x, (uint32_t)y, 1280, 720);
		} else {
			zwlr_virtual_pointer_v1_button(vp, t, 0x110 /* BTN_LEFT */,
				!strcmp(cmd, "press") ? WL_POINTER_BUTTON_STATE_PRESSED
					: WL_POINTER_BUTTON_STATE_RELEASED);
		}
		zwlr_virtual_pointer_v1_frame(vp);
		wl_display_roundtrip(dpy);
		return;
	}
	if (!locator && strcmp(cmd, "overlay") && strcmp(cmd, "passthru")) {
		LOG("no locator bound");
		return;
	}
	if (!strcmp(cmd, "at") && sscanf(line, "%*s %d %d", &x, &y) == 2) {
		zyantrik_window_locator_v1_at(locator, serial_next++, x, y);
	} else if (!strcmp(cmd, "focused")) {
		zyantrik_window_locator_v1_focused(locator, serial_next++);
	} else if (!strcmp(cmd, "capture") && sscanf(line, "%*s %31s %u %u", a, &w, &h) == 3) {
		uint32_t id = !strcmp(a, "last") ? last_id : (uint32_t)strtoul(a, NULL, 10);
		zyantrik_window_locator_v1_capture(locator, id, w, h);
	} else if (!strcmp(cmd, "flood") && sscanf(line, "%*s %d %d %d", &n, &x, &y) == 3) {
		for (int i = 0; i < n; i++) {
			zyantrik_window_locator_v1_at(locator, serial_next++, x, y);
		}
	} else if (!strcmp(cmd, "wait") && sscanf(line, "%*s %d", &n) == 1) {
		long long end = ms() + n;
		while (ms() < end) {
			wl_display_flush(dpy);
			struct pollfd pfd = { wl_display_get_fd(dpy), POLLIN, 0 };
			if (poll(&pfd, 1, (int)(end - ms())) > 0) {
				wl_display_dispatch(dpy);
			}
		}
		return;
	} else if (!strcmp(cmd, "overlay") && sscanf(line, "%*s %31s", a) == 1) {
		overlay_passthru = 0;
		overlay_set(!strcmp(a, "on"));
	} else if (!strcmp(cmd, "passthru") && sscanf(line, "%*s %31s", a) == 1) {
		overlay_passthru = 1;
		overlay_set(!strcmp(a, "on"));
	} else {
		LOG("unknown command");
		return;
	}
	wl_display_roundtrip(dpy);
}

int
main(void)
{
	const char *path = getenv("LOCATOR_PROBE_LOG");
	out = fopen(path ? path : "/tmp/locator-probe.log", "a");
	if (!out) {
		return 1;
	}
	if (getenv("LOCATOR_PROBE_CMD")) {
		cmd_path = getenv("LOCATOR_PROBE_CMD");
	}
	dpy = wl_display_connect(NULL);
	if (!dpy) {
		LOG("connect failed");
		return 1;
	}
	struct wl_registry *reg = wl_display_get_registry(dpy);
	wl_registry_add_listener(reg, &reg_listener, NULL);
	wl_display_roundtrip(dpy);
	LOG("started pid=%d: locator global %s, WAYLAND_SOCKET %s", (int)getpid(),
		locator ? "bound" : "NOT OFFERED", getenv("WAYLAND_SOCKET") ? "set" : "unset");
	for (;;) {
		wl_display_flush(dpy);
		struct pollfd pfd = { wl_display_get_fd(dpy), POLLIN, 0 };
		int r = poll(&pfd, 1, 100);
		if (r > 0 && wl_display_dispatch(dpy) == -1) {
			break;
		}
		FILE *f = fopen(cmd_path, "r");
		if (!f) {
			continue;
		}
		char line[256];
		char *lines[64];
		int count = 0;
		while (count < 64 && fgets(line, sizeof(line), f)) {
			line[strcspn(line, "\r\n")] = 0;
			lines[count++] = strdup(line);
		}
		fclose(f);
		if (unlink(cmd_path) < 0) {
			/* Never run a file twice: write it as the session's user (/tmp is sticky) */
			LOG("cannot remove %s (%s); not running it", cmd_path, strerror(errno));
			for (int i = 0; i < count; i++) {
				free(lines[i]);
			}
			sleep(5);
			continue;
		}
		for (int i = 0; i < count; i++) {
			run_line(lines[i]);
			free(lines[i]);
		}
	}
	LOG("display gone");
	return 0;
}
