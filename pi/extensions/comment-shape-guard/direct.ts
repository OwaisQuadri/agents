import { constants, type Stats } from "node:fs";
import { access, lstat, readFile, realpath, writeFile } from "node:fs/promises";
import { basename, dirname, join } from "node:path";

export type Proposal = {
	operation: "edit" | "write";
	path: string;
	repository_root: string | null;
	original_text: string | null;
	proposed_text: string;
};

type Snapshot = { identity: Stats; bytes: Buffer } | null;
type Options = {
	operation: Proposal["operation"];
	deadline: number;
	signal?: AbortSignal;
	validate: (proposal: Proposal) => Promise<void>;
};

export class ValidationCancelled extends Error {
	constructor() { super("Operation aborted before commit. Retry only if the change is still wanted."); }
}

export class ValidationExpired extends Error {
	constructor() { super("Validation deadline exceeded before commit. Retry within the validation budget."); }
}

export function checkDeadline(deadline: number, signal?: AbortSignal): void {
	if (signal?.aborted) throw new ValidationCancelled();
	if (performance.now() >= deadline) throw new ValidationExpired();
}

function sameIdentity(a: Stats, b: Stats): boolean {
	return a.dev === b.dev && a.ino === b.ino && a.mode === b.mode && a.nlink === b.nlink && a.size === b.size && a.mtimeMs === b.mtimeMs && a.ctimeMs === b.ctimeMs;
}

export function createOperations(options: Options) {
	let original: Snapshot | undefined;
	const check = () => checkDeadline(options.deadline, options.signal);
	async function checked<T>(operation: () => Promise<T>): Promise<T> {
		check();
		const result = await operation();
		check();
		return result;
	}
	async function snapshot(path: string): Promise<Snapshot> {
		let identity: Stats;
		try {
			identity = await checked(() => lstat(path));
		} catch (error) {
			if ((error as NodeJS.ErrnoException).code === "ENOENT") return null;
			throw error;
		}
		if (!identity.isFile() || identity.nlink !== 1) throw new Error("Unsupported target: require a regular file with one link.");
		const bytes = await checked(() => readFile(path));
		const after = await checked(() => lstat(path));
		if (!sameIdentity(identity, after)) throw new Error("Target changed while reading; retry the operation.");
		new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(bytes);
		return { identity, bytes };
	}
	async function repositoryRoot(path: string): Promise<string | null> {
		let directory = dirname(path);
		while (true) {
			try {
				await checked(() => lstat(join(directory, ".git")));
				return directory;
			} catch (error) {
				if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
			}
			const parent = dirname(directory);
			if (parent === directory) return null;
			directory = parent;
		}
	}
	return {
		access: (path: string) => checked(() => access(path, constants.R_OK | constants.W_OK)),
		mkdir: async (_directory: string) => { check(); },
		readFile: async (path: string): Promise<Buffer> => {
			original = await snapshot(path);
			if (!original) throw new Error("Target is missing; retry with an existing file.");
			return original.bytes;
		},
		writeFile: async (path: string, content: string): Promise<void> => {
			check();
			if (!content.isWellFormed()) throw new Error("Invalid UTF-8 candidate; correct the text before retrying.");
			const parent = await checked(() => realpath(dirname(path)));
			const canonicalPath = join(parent, basename(path));
			if (original === undefined) original = await snapshot(path);
			const initial = original;
			const root = await repositoryRoot(canonicalPath);
			await checked(() => options.validate({ operation: options.operation, path: canonicalPath, repository_root: root, original_text: initial?.bytes.toString("utf8") ?? null, proposed_text: content }));
			if (await checked(() => realpath(dirname(path))) !== parent) throw new Error("Target parent changed; retry the operation.");
			const current = await snapshot(path);
			if (initial === null ? current !== null : current === null || !sameIdentity(initial.identity, current.identity) || !initial.bytes.equals(current.bytes)) {
				throw new Error("Target changed before commit; retry the operation.");
			}
			check();
			try {
				await writeFile(path, content, { encoding: "utf8", flag: initial === null ? "wx" : "w" });
			} catch {
				throw new Error("Filesystem write failed after validation; inspect the target before retrying. Disk may have changed.");
			}
		},
	};
}
