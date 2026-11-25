/**
 * `/yantrik-room`: make this Pi process read ~/.pi/agent/models.json and settings.json again.
 *
 * The desktop rewrites those files when the model picked in the ask bar changes (its context
 * window, answer length and compaction numbers are the picked model's room, #1402). Pi reads them
 * only at its start, and Pi runs with `--no-session`, so a restart would empty the conversation.
 * Measured on Pi 0.99.1: the registry's `refresh()` alone changes nothing, `reload()` alone changes
 * nothing, and the two in this order give the running session the new `contextWindow` and the new
 * compaction numbers with its messages kept. The harness sends this as a prompt between turns,
 * never during one (yantrik_pi.py, `pi_room.RoomReload`); a prompt that is a command starts no run.
 */
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

export default function (pi: ExtensionAPI) {
	pi.registerCommand("yantrik-room", {
		description: "Read Pi's model and compaction files again (the harness sends this between turns)",
		handler: async (_args, ctx) => {
			await ctx.modelRegistry.refresh();
			await ctx.reload();
		},
	});
}
