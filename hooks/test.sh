#!/usr/bin/env bash
set -u
HOOK="$(cd "$(dirname "$0")" && pwd)/post-checkout"
T=/tmp/hooktest2/scratch
FAILS=0

check() {
  local label="$1"; shift
  if "$@"; then echo "PASS $label"; else echo "FAIL $label"; FAILS=$((FAILS+1)); fi
}

setup() {
  rm -rf "$T"; mkdir -p "$T/live"; cd "$T/live" || exit 1
  git init -qb main . && git config user.email t@t && git config user.name t
  mkdir hooks sub
  cp "$HOOK" hooks/post-checkout && chmod +x hooks/post-checkout
  printf 'telemetry/\n' > .gitignore
  echo v1 > tracked.txt; echo base > sub/b.txt
  git add -A && git commit -qm c1
  git branch old
  echo v2 > tracked.txt && git commit -qam c2
  ln -s "$T/live/hooks/post-checkout" .git/hooks/post-checkout
}

dirty_live() {
  echo v3-live-edit > tracked.txt
  mkdir -p newskill telemetry
  echo skill > newskill/SKILL.md
  echo log > telemetry/usage.jsonl
}

echo "=== T1 worktree add from main tip: carries modified + untracked, skips ignored"
setup; dirty_live
git worktree add -q ../wt1 -b feat-a main
check "T1 exit-0 creation" test -d ../wt1
check "T1 modified carried" grep -q v3-live-edit ../wt1/tracked.txt
check "T1 untracked carried" test -f ../wt1/newskill/SKILL.md
check "T1 ignored skipped" test ! -e ../wt1/telemetry

echo "=== T2 worktree add from old branch: carries nothing"
setup; dirty_live
git worktree add -q ../wt2 -b feat-b old
check "T2 tracked at v1" grep -q v1 ../wt2/tracked.txt
check "T2 no untracked" test ! -e ../wt2/newskill

echo "=== T3 branch from main inside a clean worktree: carries"
setup; dirty_live
git worktree add -q ../wt3 -b feat-c old
git -C ../wt3 switch -qc feat-d main
check "T3 modified carried" grep -q v3-live-edit ../wt3/tracked.txt
check "T3 untracked carried" test -f ../wt3/newskill/SKILL.md

echo "=== T4 live checkout never written"
check "T4 live tracked intact" grep -q v3-live-edit "$T/live/tracked.txt"
check "T4 live status stable" test "$(git -C "$T/live" status --porcelain | wc -l | tr -d ' ')" = 2

echo "=== T5 worktree cut at origin/main tip when it is ahead of local main"
setup; dirty_live
git update-ref refs/remotes/origin/main "$(echo c3 | git commit-tree -p main "$(git rev-parse 'main^{tree}')")"
git worktree add -q ../wt5 -b feat-e origin/main
check "T5 modified carried" grep -q v3-live-edit ../wt5/tracked.txt

echo "=== T6 flag-0 file checkout: no-op, no crash"
setup; dirty_live
git worktree add -q ../wt6 -b feat-f old
git -C ../wt6 checkout -q -- tracked.txt
check "T6 exit 0" test $? -eq 0

echo "=== T7 deleted tracked file in live: creation succeeds, deletion propagates"
setup; dirty_live
rm sub/b.txt
git worktree add -q ../wt7 -b feat-g main
check "T7 creation exit 0" test $? -eq 0
check "T7 deletion propagated" test ! -e ../wt7/sub/b.txt
check "T7 other files carried" test -f ../wt7/newskill/SKILL.md

echo "=== T8 dangling untracked symlink in live: skipped, rest carried, exit 0"
setup; dirty_live
ln -s /nonexistent/target aaa-dangling
ln -s tracked.txt zzz-valid-link
git worktree add -q ../wt8 -b feat-h main 2>"$T/t8.err"
check "T8 creation exit 0" test $? -eq 0
check "T8 dangling skipped" test ! -L ../wt8/aaa-dangling
check "T8 valid link carried" test -L ../wt8/zzz-valid-link
check "T8 untracked carried" test -f ../wt8/newskill/SKILL.md
check "T8 no rsync errors" bash -c '! grep -Ev "Applied patch|Falling back" "$0" | grep -q .' "$T/t8.err"

echo "=== T9 staged new file and staged edit: carried"
setup
echo staged-content > staged-new.md && git add staged-new.md
echo v3-staged-edit > tracked.txt && git add tracked.txt
git worktree add -q ../wt9 -b feat-i main
check "T9 staged new carried" grep -q staged-content ../wt9/staged-new.md
check "T9 staged edit carried" grep -q v3-staged-edit ../wt9/tracked.txt

echo "=== T10 dirty destination: later checkout at tip must not clobber"
setup; dirty_live
git worktree add -q ../wt10 -b feat-j main
echo workspace-newer > ../wt10/tracked.txt
git -C ../wt10 switch -qc feat-k main
check "T10 workspace edit preserved" grep -q workspace-newer ../wt10/tracked.txt

echo "=== T11 rebase onto main with dirty live: no injection, rebase completes"
setup
git worktree add -q ../wt11 -b feat-l old
echo feature-work > ../wt11/feature.txt
git -C ../wt11 add feature.txt && git -C ../wt11 commit -qm feat
dirty_live
git -C ../wt11 rebase -q main 2>"$T/t11.err"
rebase_status=$?
check "T11 rebase exit 0" test $rebase_status -eq 0
check "T11 no live dirt injected" test ! -e ../wt11/newskill
check "T11 tree clean after rebase" test -z "$(git -C ../wt11 status --porcelain)"
check "T11 commit survived" bash -c 'git -C "$0" log --oneline -1 | grep -q feat' "$T/wt11"

echo "=== T12 live behind origin/main: leaked GIT_DIR must not corrupt the listing"
setup; dirty_live
upstream_tree_dir="$T/upstream" && mkdir -p "$upstream_tree_dir"
git worktree add -q "$upstream_tree_dir" --detach main
echo brand-new-upstream > "$upstream_tree_dir/brand-new.txt"
echo fresh-upstream-b > "$upstream_tree_dir/sub/b.txt"
git -C "$upstream_tree_dir" add -A && git -C "$upstream_tree_dir" commit -qm c3
git update-ref refs/remotes/origin/main "$(git -C "$upstream_tree_dir" rev-parse HEAD)"
git worktree remove -f "$upstream_tree_dir"
git worktree add -q ../wt12 -b feat-m origin/main
git -C ../wt12 switch -qc feat-n origin/main 2>"$T/t12.err"
check "T12 switch exit 0" test $? -eq 0
check "T12 upstream file untouched" grep -q brand-new-upstream ../wt12/brand-new.txt
check "T12 no rsync stat errors" bash -c '! grep -q "No such file" "$0"' "$T/t12.err"

echo "=== T13 file dirty in live AND changed upstream: merged, not silently reverted"
setup
printf 'line1\nline2\nline3\n' > sub/b.txt && git add sub/b.txt && git commit -qm c3
upstream_tree_dir="$T/upstream2" && mkdir -p "$upstream_tree_dir"
git worktree add -q "$upstream_tree_dir" --detach main
printf 'line1-upstream-PR\nline2\nline3\n' > "$upstream_tree_dir/sub/b.txt"
git -C "$upstream_tree_dir" add -A && git -C "$upstream_tree_dir" commit -qm c4
git update-ref refs/remotes/origin/main "$(git -C "$upstream_tree_dir" rev-parse HEAD)"
git worktree remove -f "$upstream_tree_dir"
printf 'line1\nline2\nline3-LIVE-EDIT\n' > sub/b.txt
git worktree add -q ../wt13 -b feat-o origin/main
check "T13 upstream hunk kept" grep -q line1-upstream-PR ../wt13/sub/b.txt
check "T13 live edit applied" grep -q line3-LIVE-EDIT ../wt13/sub/b.txt

echo "=== T14 no-args invocation: no-op in live, carries in a clean fresh worktree"
setup; dirty_live
git worktree add -q ../wt14 --no-checkout -b feat-p old
rm .git/hooks/post-checkout
git -C ../wt14 checkout -q feat-p
git -C ../wt14 switch -qc feat-q main
ln -s "$T/live/hooks/post-checkout" .git/hooks/post-checkout
( cd ../wt14 && ./hooks/post-checkout 2>/dev/null || ../live/hooks/post-checkout )
check "T14 no-args carried" test -f ../wt14/newskill/SKILL.md
( cd "$T/live" && ./hooks/post-checkout )
check "T14 live no-op exit 0" test $? -eq 0
check "T14 live still intact" grep -q v3-live-edit "$T/live/tracked.txt"

echo "=== T15 live stopped at a merge conflict: nothing carried, no marker pollution"
setup
git switch -qc clash main
echo clash-edit > tracked.txt && git commit -qam clash
git switch -q main
echo main-edit > tracked.txt && git commit -qam mainedit
git merge -q clash >/dev/null 2>&1 || true
check "T15 live is mid-conflict" test -e .git/MERGE_HEAD
git worktree add -q ../wt15 -b feat-r main 2>"$T/t15.err"
check "T15 creation exit 0" test $? -eq 0
check "T15 no markers carried" bash -c '! grep -q "<<<<<<<" "$0"' ../wt15/tracked.txt
check "T15 wt at committed content" grep -q main-edit ../wt15/tracked.txt

echo "=== T16 modified tracked binary (sorts last): carried byte-for-byte"
setup
printf 'BIN\x00HEADER-v1' > zz.bin && git add zz.bin && git commit -qm addbin
printf 'BIN\x00HEADER-v2-edited' > zz.bin
git worktree add -q ../wt16 -b feat-s main 2>"$T/t16.err"
check "T16 creation exit 0" test $? -eq 0
check "T16 binary identical" cmp -s zz.bin ../wt16/zz.bin
check "T16 no corrupt-patch error" bash -c '! grep -q corrupt "$0"' "$T/t16.err"

echo "=== T17 NUL past the 8000-byte text sniff: carried byte-for-byte"
setup
{ printf 'a%.0s' $(seq 1 8100); printf '\ntail-v1\n'; } > data.log
git add data.log && git commit -qm addlog
{ printf 'a%.0s' $(seq 1 8100); printf '\ntail-v2\x00after-nul\n'; } > data.log
git worktree add -q ../wt17 -b feat-t main
check "T17 nul file identical" cmp -s data.log ../wt17/data.log

echo "=== T18 carried tracked changes land unstaged"
setup; dirty_live
git worktree add -q ../wt18 -b feat-u main
check "T18 tracked change unstaged" bash -c 'git -C "$0" status --porcelain | grep -q "^ M tracked.txt"' ../wt18
check "T18 nothing staged" bash -c 'test -z "$(git -C "$0" diff --cached --name-only)"' ../wt18

echo "=== T19 switch --orphan: unborn HEAD, exit 0"
setup; dirty_live
git worktree add -q ../wt19 -b feat-v old
git -C ../wt19 switch -q --orphan blank 2>"$T/t19.err"
check "T19 orphan switch exit 0" test $? -eq 0
check "T19 no fatal in stderr" bash -c '! grep -q fatal "$0"' "$T/t19.err"

echo
if [[ $FAILS -eq 0 ]]; then echo "ALL PASS"; else echo "$FAILS FAILURES"; exit 1; fi
