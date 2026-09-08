/**
 * Builds the preferred command checker's arguments for one Bash tool call.
 *
 * @param command The exact Bash command.
 * @param timeout The optional Bash tool timeout in seconds.
 * @param repositoryRoot The repository root that owns evaluation runners.
 * @returns The checker arguments.
 * @throws Never.
 */
export function preferredCliGuardArguments(
	command: string,
	timeout: number | undefined,
	repositoryRoot: string,
): string[] {
	const args = ["--check", command];
	if (timeout !== undefined) args.push("--timeout", String(timeout));
	args.push("--repository-root", repositoryRoot);
	return args;
}
