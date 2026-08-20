import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

/**
 * Live request and overall diffs: snapshot at request start, badge refresh
 * after write-capable tools and on settle, /diff overlay with in-place hunk
 * folding and open-in-nvim.
 *
 * @param pi extension API
 */
export default function liveDiff(pi: ExtensionAPI): void {
	throw new Error("unimplemented");
}
