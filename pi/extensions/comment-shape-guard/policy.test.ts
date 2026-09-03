import assert from "node:assert/strict";
import { test } from "node:test";

import { editFragments, extensionOf, followingContextOnDisk, followingContextWithinFragment, writeFragment } from "./policy.ts";

test("editFragments pairs each edits[] entry with the file path", () => {
	const fragments = editFragments({
		path: "/repo/src/lib.rs",
		edits: [
			{ oldText: "fn a() {}", newText: "// note\nfn a() {}" },
			{ oldText: "fn b() {}", newText: "fn b() { 1 }" },
		],
	});
	assert.equal(fragments.length, 2);
	assert.equal(fragments[0].path, "/repo/src/lib.rs");
	assert.equal(fragments[0].newText, "// note\nfn a() {}");
	assert.equal(fragments[1].oldText, "fn b() {}");
});

test("writeFragment carries the whole new content", () => {
	const fragment = writeFragment({ path: "/repo/src/new.rs", content: "fn main() {}\n" });
	assert.equal(fragment.content, "fn main() {}\n");
});

test("extensionOf reads the extension off the file name, not the whole path", () => {
	assert.equal(extensionOf("/repo/src/lib.rs"), "rs");
	assert.equal(extensionOf("install.sh"), "sh");
	assert.equal(extensionOf("/repo/config.toml"), "toml");
});

test("extensionOf returns undefined for an extensionless path or a bare dotfile", () => {
	assert.equal(extensionOf("/repo/Makefile"), undefined);
	assert.equal(extensionOf(".gitignore"), undefined);
});

test("followingContextWithinFragment returns the non-blank lines after a span in the same fragment", () => {
	const fragment = "/// docstring\npub fn public_fn() {}\n";
	const context = followingContextWithinFragment(fragment, { startLine: 1, endLine: 1, kind: "doc", text: "/// docstring" });
	assert.equal(context, "pub fn public_fn() {}");
});

test("followingContextWithinFragment is empty when the span is the fragment's last line", () => {
	const fragment = "fn a() {}\n// trailing note";
	const context = followingContextWithinFragment(fragment, { startLine: 2, endLine: 2, kind: "plain", text: "// trailing note" });
	assert.equal(context, "");
});

test("followingContextOnDisk finds what follows oldText in the current file", () => {
	const currentFile = "fn a() {}\n\nfn to_replace() {}\n\nfn c() { 1 }\n";
	const context = followingContextOnDisk(currentFile, "fn to_replace() {}");
	assert.equal(context, "fn c() { 1 }");
});

test("followingContextOnDisk returns undefined when oldText cannot be located", () => {
	const currentFile = "fn a() {}\n";
	assert.equal(followingContextOnDisk(currentFile, "fn missing() {}"), undefined);
});

test("followingContextOnDisk returns undefined when oldText is the file's last content — genuinely unverifiable", () => {
	const currentFile = "fn a() {}\n\nfn last() {}\n";
	assert.equal(followingContextOnDisk(currentFile, "fn last() {}"), undefined);
});
