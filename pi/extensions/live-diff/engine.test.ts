import { test } from "node:test";
import assert from "node:assert/strict";
import { execFile, execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import * as fs from "node:fs";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { promisify } from "node:util";

import { captureSnapshot, diffStats, filePatch, TEMP_INDEX_PREFIX } from "./engine.ts";
import type { Exec } from "./engine.ts";
import {
	addBinaryFile,
	addIgnoredFile,
	addRename,
	addStagedEdit,
	addUnstagedEdit,
	addUntrackedFile,
	FIXTURE_PREFIX,
	makeFixtureRepo,
} from "./fixtures.ts";
import type { FixtureRepo } from "./fixtures.ts";

const execFileAsync = promisify(execFile);

const exec: Exec = async (command, args, options) => {
	try {
		const { stdout, stderr } = await execFileAsync(command, args, {
			cwd: options?.cwd,
			env: options?.env ? { ...process.env, ...options.env } : process.env,
			encoding: "utf8",
			maxBuffer: 64 * 1024 * 1024,
		});
		return { code: 0, stdout, stderr };
	} catch (error) {
		const failure = error as { code?: number; stdout?: string; stderr?: string };
		return {
			code: typeof failure.code === "number" ? failure.code : 1,
			stdout: failure.stdout ?? "",
			stderr: failure.stderr ?? "",
		};
	}
};

function indexSha256(repo: FixtureRepo): string {
	return createHash("sha256")
		.update(fs.readFileSync(path.join(repo.root, ".git", "index")))
		.digest("hex");
}

function statusZ(repo: FixtureRepo): string {
	return repo.git(["status", "--porcelain", "-z"]);
}

function refLineCount(repo: FixtureRepo): number {
	const output = repo.git(["for-each-ref"]);
	return output === "" ? 0 : output.trimEnd().split("\n").length;
}

function listFilesRecursive(root: string): string[] {
	return fs
		.readdirSync(root, { recursive: true, encoding: "utf8" })
		.filter((entry) => {
			const parts = entry.split(path.sep);
			return parts[0] !== ".git" && fs.statSync(path.join(root, entry)).isFile();
		})
		.sort();
}

test("TC-01 snapshot is a tree and repo state is untouched", async (t) => {
	const repo = makeFixtureRepo(tmpdir());
	t.after(() => repo.cleanup());
	addStagedEdit(repo);
	addUnstagedEdit(repo);
	addUntrackedFile(repo);
	const indexBefore = indexSha256(repo);
	const statusBefore = statusZ(repo);
	const refsBefore = refLineCount(repo);

	const snapshot = await captureSnapshot(exec, repo.root);

	const objectType = repo.git(["cat-file", "-t", snapshot.treeSha]).trim();
	assert.equal(objectType, "tree");
	assert.equal(indexSha256(repo), indexBefore);
	assert.equal(statusZ(repo), statusBefore);
	assert.equal(refLineCount(repo), refsBefore);
});

test("TC-02 five change classes classified, ignored file absent", async (t) => {
	const repo = makeFixtureRepo(tmpdir());
	t.after(() => repo.cleanup());
	const ignored = addIgnoredFile(repo);
	const snapshot = await captureSnapshot(exec, repo.root);
	const staged = addStagedEdit(repo);
	const unstaged = addUnstagedEdit(repo);
	const untracked = addUntrackedFile(repo);
	const rename = addRename(repo);
	const binary = addBinaryFile(repo);

	const stats = await diffStats(exec, repo.root, snapshot.treeSha, 100);
	const byPath = new Map(stats.files.map((file) => [file.path, file]));

	assert.equal(stats.files.length, 5);
	assert.equal(byPath.get(staged)?.kind, "modified");
	assert.equal(byPath.get(unstaged)?.kind, "modified");
	assert.equal(byPath.get(untracked)?.kind, "added");
	assert.equal(byPath.get(rename.to)?.kind, "renamed");
	assert.equal(byPath.get(rename.to)?.renamedFrom, rename.from);
	assert.equal(byPath.get(binary)?.isBinary, true);
	assert.equal(byPath.get(binary)?.additions, 0);
	assert.equal(byPath.has(ignored), false);
});

test("TC-03 request vs overall separation", async (t) => {
	const repo = makeFixtureRepo(tmpdir());
	t.after(() => repo.cleanup());
	const preexisting = addUnstagedEdit(repo);
	const snapshot = await captureSnapshot(exec, repo.root);
	const postSnapshot = addStagedEdit(repo);

	const request = await diffStats(exec, repo.root, snapshot.treeSha, 100);
	const overall = await diffStats(exec, repo.root, snapshot.baselineSha, 100);

	assert.deepEqual(
		request.files.map((file) => file.path),
		[postSnapshot],
	);
	assert.deepEqual(
		overall.files.map((file) => file.path).sort(),
		[preexisting, postSnapshot].sort(),
	);
});

test("TC-09 empty change set gives zero rows both modes", async (t) => {
	const repo = makeFixtureRepo(tmpdir());
	t.after(() => repo.cleanup());
	const snapshot = await captureSnapshot(exec, repo.root);

	const request = await diffStats(exec, repo.root, snapshot.treeSha, 100);
	const overall = await diffStats(exec, repo.root, snapshot.baselineSha, 100);

	assert.equal(request.files.length, 0);
	assert.equal(overall.files.length, 0);
});

test("TC-10 truncation at the cap", async (t) => {
	const repo = makeFixtureRepo(tmpdir());
	t.after(() => repo.cleanup());
	const snapshot = await captureSnapshot(exec, repo.root);
	for (let i = 0; i < 15; i += 1) {
		addUntrackedFile(repo, `bulk-${String(i).padStart(2, "0")}.txt`);
	}

	const stats = await diffStats(exec, repo.root, snapshot.treeSha, 10);

	assert.equal(stats.files.length, 10);
	assert.equal(stats.isTruncated, true);
});

test("TC-11 repo with zero commits", async (t) => {
	const root = fs.mkdtempSync(path.join(tmpdir(), "live-diff-zero-"));
	t.after(() => fs.rmSync(root, { recursive: true, force: true }));
	execFileSync("git", ["init", "-q"], { cwd: root });
	execFileSync("git", ["config", "user.name", "fixture"], { cwd: root });
	execFileSync("git", ["config", "user.email", "fixture@example.invalid"], {
		cwd: root,
	});
	fs.writeFileSync(path.join(root, "only.txt"), "only content\n");
	const emptyTreeSha = execFileSync(
		"git",
		["hash-object", "-t", "tree", "/dev/null"],
		{ cwd: root, encoding: "utf8" },
	).trim();

	const snapshot = await captureSnapshot(exec, root);
	const stats = await diffStats(exec, root, snapshot.baselineSha, 100);

	assert.equal(snapshot.baselineSha, emptyTreeSha);
	assert.equal(stats.files.length, 1);
	assert.equal(stats.files[0].path, "only.txt");
	assert.equal(stats.files[0].kind, "added");
});

test("TC-16 hostile file names stay single rows without execution", async (t) => {
	const repo = makeFixtureRepo(tmpdir());
	t.after(() => repo.cleanup());
	const snapshot = await captureSnapshot(exec, repo.root);
	const escName = `esc-\u001b]0;x\u0007.txt`;
	const injectionName = "$(touch /tmp/pwned-agnt0015).txt";
	fs.writeFileSync(path.join(repo.root, escName), "esc content\n");
	fs.mkdirSync(path.dirname(path.join(repo.root, injectionName)), {
		recursive: true,
	});
	fs.writeFileSync(path.join(repo.root, injectionName), "injection content\n");

	const stats = await diffStats(exec, repo.root, snapshot.treeSha, 100);
	const paths = stats.files.map((file) => file.path).sort();

	assert.equal(stats.files.length, 2);
	assert.deepEqual(paths, [injectionName, escName].sort());
	assert.equal(fs.existsSync("/tmp/pwned-agnt0015"), false);
});

test("TC-18 engine writes nothing into the worktree", async (t) => {
	const repo = makeFixtureRepo(tmpdir());
	t.after(() => repo.cleanup());
	const edited = addUnstagedEdit(repo);
	const snapshot = await captureSnapshot(exec, repo.root);
	const listingBefore = listFilesRecursive(repo.root);

	await captureSnapshot(exec, repo.root);
	await diffStats(exec, repo.root, snapshot.treeSha, 100);
	await filePatch(exec, repo.root, snapshot.baselineSha, edited);

	assert.deepEqual(listFilesRecursive(repo.root), listingBefore);
});

test("a snapshot sweeps stale temp index dirs and spares fresh ones", async (t) => {
	const repo = makeFixtureRepo(tmpdir());
	t.after(() => repo.cleanup());
	const staleDir = fs.mkdtempSync(path.join(tmpdir(), TEMP_INDEX_PREFIX));
	const freshDir = fs.mkdtempSync(path.join(tmpdir(), TEMP_INDEX_PREFIX));
	const unrelatedDir = fs.mkdtempSync(path.join(tmpdir(), "live-diff-fixture-"));
	fs.writeFileSync(path.join(staleDir, "index"), "stale\n");
	const staleTime = new Date(Date.now() - 2 * 60 * 60 * 1000);
	fs.utimesSync(staleDir, staleTime, staleTime);
	fs.utimesSync(unrelatedDir, staleTime, staleTime);

	const snapshot = await captureSnapshot(exec, repo.root);

	assert.equal(fs.existsSync(staleDir), false);
	assert.equal(fs.existsSync(freshDir), true);
	assert.equal(fs.existsSync(unrelatedDir), true);
	assert.equal(repo.git(["cat-file", "-t", snapshot.treeSha]).trim(), "tree");

	fs.rmSync(freshDir, { recursive: true, force: true });
	fs.rmSync(unrelatedDir, { recursive: true, force: true });
});

test("F-11 makeFixtureRepo sweeps stale fixture dirs and spares fresh ones", (t) => {
	const staleDir = fs.mkdtempSync(path.join(tmpdir(), FIXTURE_PREFIX));
	const freshDir = fs.mkdtempSync(path.join(tmpdir(), FIXTURE_PREFIX));
	fs.writeFileSync(path.join(staleDir, "leftover.txt"), "stale\n");
	const staleTime = new Date(Date.now() - 2 * 60 * 60 * 1000);
	fs.utimesSync(staleDir, staleTime, staleTime);
	t.after(() => fs.rmSync(freshDir, { recursive: true, force: true }));

	const repo = makeFixtureRepo(tmpdir());
	t.after(() => repo.cleanup());

	assert.equal(fs.existsSync(staleDir), false);
	assert.equal(fs.existsSync(freshDir), true);
});

test("F-11 fixture cleanup removes its root and is safe to repeat", () => {
	const repo = makeFixtureRepo(tmpdir());
	assert.equal(fs.existsSync(repo.root), true);

	repo.cleanup();

	assert.equal(fs.existsSync(repo.root), false);
	repo.cleanup();
});

test("filePatch returns hunks for text and [] for binary", async (t) => {
	const repo = makeFixtureRepo(tmpdir());
	t.after(() => repo.cleanup());
	const snapshot = await captureSnapshot(exec, repo.root);
	const edited = addUnstagedEdit(repo);
	const binary = addBinaryFile(repo);

	const textHunks = await filePatch(exec, repo.root, snapshot.treeSha, edited);
	const binaryHunks = await filePatch(exec, repo.root, snapshot.treeSha, binary);

	assert.ok(textHunks.length > 0);
	for (const hunk of textHunks) {
		assert.match(hunk.header, /^@@ /);
		for (const line of hunk.lines) {
			assert.ok([" ", "+", "-"].includes(line.origin));
		}
	}
	assert.deepEqual(binaryHunks, []);
});
