import {
	CustomEditor,
	type ExtensionAPI,
	type ExtensionContext,
} from "@earendil-works/pi-coding-agent";
import type { TUI } from "@earendil-works/pi-tui";

import { getPlannotatorPhase, renderPlanningBorder } from "./plannotator-plan-border/policy.ts";

type EditorFactory = NonNullable<ReturnType<ExtensionContext["ui"]["getEditorComponent"]>>;

/**
 * Registers a phase-aware dotted border around the existing Pi editor.
 * @param pi The Pi extension interface used to register session handlers.
 * @returns Nothing.
 * @throws Never.
 */
export default function plannotatorPlanBorder(pi: ExtensionAPI): void {
	let activeContext: ExtensionContext | undefined;
	let activeTui: TUI | undefined;
	let previousEditorFactory: EditorFactory | undefined;
	let installedEditorFactory: EditorFactory | undefined;
	let restoreEditorRender: (() => void) | undefined;
	let sessionGeneration = 0;
	let cachedLeafId: string | null | undefined;
	let cachedPhase: ReturnType<typeof getPlannotatorPhase> = "idle";

	const requestRender = (): void => activeTui?.requestRender();
	const readPhase = (ctx: ExtensionContext): ReturnType<typeof getPlannotatorPhase> => {
		try {
			const leafId = ctx.sessionManager.getLeafId();
			if (leafId !== cachedLeafId) {
				cachedLeafId = leafId;
				cachedPhase = getPlannotatorPhase(ctx.sessionManager.getBranch());
			}
			return cachedPhase;
		} catch {
			cachedLeafId = undefined;
			cachedPhase = "idle";
			return cachedPhase;
		}
	};

	pi.on("session_start", (_event, ctx) => {
		const generation = ++sessionGeneration;
		activeContext = ctx;
		cachedLeafId = undefined;
		cachedPhase = "idle";
		setTimeout(() => {
			if (
				generation !== sessionGeneration ||
				ctx.mode !== "tui" ||
				typeof ctx.ui.getEditorComponent !== "function" ||
				typeof ctx.ui.setEditorComponent !== "function"
			) return;

			const currentEditorFactory = ctx.ui.getEditorComponent();
			if (installedEditorFactory !== undefined && currentEditorFactory === installedEditorFactory) return;
			previousEditorFactory = currentEditorFactory;
			const baseEditorFactory = currentEditorFactory;
			installedEditorFactory = (tui, theme, keybindings) => {
				activeTui = tui;
				restoreEditorRender?.();
				const editor = baseEditorFactory?.(tui, theme, keybindings) ?? new CustomEditor(
					tui,
					theme,
					keybindings,
					{ embedWorkingStatus: true },
				);
				const originalRender = editor.render;
				const wrappedRender = (width: number) => renderPlanningBorder(
					originalRender.call(editor, width),
					activeContext === undefined ? "idle" : readPhase(activeContext),
				);
				editor.render = wrappedRender;
				restoreEditorRender = () => {
					if (editor.render === wrappedRender) editor.render = originalRender;
				};
				return editor;
			};
			ctx.ui.setEditorComponent(installedEditorFactory);
		}, 0);
	});

	pi.on("message_end", requestRender);
	pi.on("tool_result", requestRender);
	pi.on("agent_end", requestRender);
	pi.on("session_tree", requestRender);

	pi.on("session_shutdown", (_event, ctx) => {
		sessionGeneration += 1;
		if (
			installedEditorFactory !== undefined &&
			typeof ctx.ui.getEditorComponent === "function" &&
			typeof ctx.ui.setEditorComponent === "function" &&
			ctx.ui.getEditorComponent() === installedEditorFactory
		) {
			ctx.ui.setEditorComponent(previousEditorFactory);
		}
		restoreEditorRender?.();
		activeContext = undefined;
		activeTui = undefined;
		previousEditorFactory = undefined;
		installedEditorFactory = undefined;
		restoreEditorRender = undefined;
		cachedLeafId = undefined;
		cachedPhase = "idle";
	});
}
