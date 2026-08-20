import { execFileSync } from "node:child_process";
import * as fs from "node:fs";
import * as path from "node:path";

export const FIXTURE_PREFIX = "live-diff-fixture-";

const STALE_FIXTURE_AGE_MS = 60 * 60 * 1000;

export interface FixtureRepo {
	root: string;
	git: (args: string[]) => string;
	cleanup: () => void;
}

/**
 * Build a throwaway git repository with a committed three-file baseline.
 *
 * @param tmpdir directory to create the repository under (mkdtemp prefix dir)
 * @returns FixtureRepo with the repo root, a git runner bound to it, and a
 *   cleanup that removes the root
 * @throws Error when git exits nonzero or the directory cannot be created
 */
export function makeFixtureRepo(tmpdir: string): FixtureRepo {
	sweepStaleFixtureDirs(tmpdir);
	const root = fs.mkdtempSync(path.join(tmpdir, FIXTURE_PREFIX));
	const git = (args: string[]): string =>
		execFileSync("git", args, { cwd: root, encoding: "utf8" });
	const cleanup = (): void => {
		try {
			fs.rmSync(root, { recursive: true, force: true });
		} catch {
			return;
		}
	};
	git(["init", "-q"]);
	git(["config", "user.name", "fixture"]);
	git(["config", "user.email", "fixture@example.invalid"]);
	fs.writeFileSync(path.join(root, "alpha.txt"), "alpha line one\nalpha line two\n");
	fs.writeFileSync(path.join(root, "beta.txt"), "beta line one\nbeta line two\n");
	fs.writeFileSync(path.join(root, "gamma.txt"), "gamma line one\ngamma line two\n");
	git(["add", "-A"]);
	git(["commit", "-q", "-m", "baseline"]);
	return { root, git, cleanup };
}

function sweepStaleFixtureDirs(tmpdir: string): void {
	const cutoff = Date.now() - STALE_FIXTURE_AGE_MS;
	let entries: string[];
	try {
		entries = fs.readdirSync(tmpdir);
	} catch {
		return;
	}
	for (const entry of entries) {
		if (!entry.startsWith(FIXTURE_PREFIX)) {
			continue;
		}
		const target = path.join(tmpdir, entry);
		try {
			const info = fs.statSync(target);
			if (!info.isDirectory() || info.mtimeMs >= cutoff) {
				continue;
			}
			fs.rmSync(target, { recursive: true, force: true });
		} catch {
			continue;
		}
	}
}

/**
 * Branch off the repository's default branch and commit one new file on it.
 *
 * @param repo fixture repository
 * @param branch branch name to create, default "feature"
 * @param name file to create and commit on the branch, default "feature.txt"
 * @returns the committed file path relative to the repo root
 * @throws Error when git exits nonzero
 */
export function addBranchCommit(
	repo: FixtureRepo,
	branch = "feature",
	name = "feature.txt",
): string {
	repo.git(["checkout", "-q", "-b", branch]);
	fs.writeFileSync(path.join(repo.root, name), "feature line one\n");
	repo.git(["add", name]);
	repo.git(["commit", "-q", "-m", `add ${name}`]);
	return name;
}

/**
 * Commit every pending change in the fixture repository.
 *
 * @param repo fixture repository
 * @param message commit message, default "commit pending work"
 * @throws Error when git exits nonzero
 */
export function commitAll(repo: FixtureRepo, message = "commit pending work"): void {
	repo.git(["add", "-A"]);
	repo.git(["commit", "-q", "-m", message]);
}

/**
 * Stage an edit to the committed baseline file alpha.txt.
 *
 * @param repo fixture repository
 * @returns the edited file path relative to the repo root
 * @throws Error when git exits nonzero
 */
export function addStagedEdit(repo: FixtureRepo): string {
	fs.appendFileSync(path.join(repo.root, "alpha.txt"), "staged edit\n");
	repo.git(["add", "alpha.txt"]);
	return "alpha.txt";
}

/**
 * Make an unstaged edit to the committed baseline file beta.txt.
 *
 * @param repo fixture repository
 * @returns the edited file path relative to the repo root
 * @throws Error when the file cannot be written
 */
export function addUnstagedEdit(repo: FixtureRepo): string {
	fs.appendFileSync(path.join(repo.root, "beta.txt"), "unstaged edit\n");
	return "beta.txt";
}

/**
 * Create an untracked text file.
 *
 * @param repo fixture repository
 * @param name file name, default "untracked.txt"
 * @returns the created file path relative to the repo root
 * @throws Error when the file cannot be written
 */
export function addUntrackedFile(repo: FixtureRepo, name = "untracked.txt"): string {
	fs.writeFileSync(path.join(repo.root, name), "untracked content\n");
	return name;
}

/**
 * Rename the committed baseline file gamma.txt with git mv and stage a small edit to it.
 *
 * @param repo fixture repository
 * @returns object with from (old path) and to (new path), repo-relative
 * @throws Error when git exits nonzero
 */
export function addRename(repo: FixtureRepo): { from: string; to: string } {
	const from = "gamma.txt";
	const to = "gamma-renamed.txt";
	repo.git(["mv", from, to]);
	fs.appendFileSync(path.join(repo.root, to), "post-rename edit\n");
	repo.git(["add", to]);
	return { from, to };
}

/**
 * Create an untracked binary file with NUL bytes inside the first 8000 bytes.
 *
 * @param repo fixture repository
 * @param name file name, default "blob.bin"
 * @returns the created file path relative to the repo root
 * @throws Error when the file cannot be written
 */
export function addBinaryFile(repo: FixtureRepo, name = "blob.bin"): string {
	const buffer = Buffer.alloc(4096);
	for (let i = 0; i < buffer.length; i += 2) {
		buffer[i] = 0;
		buffer[i + 1] = 0xff;
	}
	fs.writeFileSync(path.join(repo.root, name), buffer);
	return name;
}

/**
 * Commit a .gitignore entry and write the file it ignores.
 *
 * @param repo fixture repository
 * @returns the ignored file path relative to the repo root
 * @throws Error when git exits nonzero
 */
export function addIgnoredFile(repo: FixtureRepo): string {
	const name = "ignored.log";
	fs.appendFileSync(path.join(repo.root, ".gitignore"), `${name}\n`);
	repo.git(["add", ".gitignore"]);
	repo.git(["commit", "-q", "-o", "-m", "ignore rule", "--", ".gitignore"]);
	fs.writeFileSync(path.join(repo.root, name), "ignored content\n");
	return name;
}
