// TODO(AGNT-0066.T03): Register the read-only global state command and its detail arguments.
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

/**
 * Registers Pi's read-only Herdr state command.
 *
 * @param pi The Pi extension application programming interface.
 * @returns Nothing.
 * @throws Error until the command implementation is added.
 */
export default function herdrState(_pi: ExtensionAPI): void {
	throw new Error("unimplemented");
}
