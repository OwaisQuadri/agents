#!/usr/bin/env bash
set -u
HOOK="$(cd "$(dirname "$0")" && pwd)/post-checkout"
T="$(mktemp -d "${TMPDIR:-/tmp}/post-checkout-test.XXXXXX")"
[[ -n "$T" && -d "$T" ]] || exit 1
FAILS=0
trap 'rm -rf "$T"' EXIT

check() {
  local label="$1"; shift
  if "$@"; then echo "PASS $label"; else echo "FAIL $label"; FAILS=$((FAILS+1)); fi
}

setup() {
  mkdir -p "$T/live"; cd "$T/live" || exit 1
  git init -qb main . && git config user.email t@t && git config user.name t
  mkdir hooks sub
  cp "$HOOK" hooks/post-checkout && chmod +x hooks/post-checkout
  printf 'telemetry/\n.install-test-home.log\n' > .gitignore
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

reset_fixture() {
  rm -rf "$T"
  mkdir -p "$T"
  setup
}

echo "=== T1 worktree add from main keeps committed content only"
reset_fixture; dirty_live
git worktree add -q ../wt1 -b feat-a main
check "T1 creation" test -d ../wt1
check "T1 committed tracked content" grep -q v2 ../wt1/tracked.txt
check "T1 no untracked copy" test ! -e ../wt1/newskill
check "T1 clean destination" test -z "$(git -C ../wt1 status --porcelain)"

echo "=== T2 staged primary changes stay in the primary checkout"
reset_fixture
echo staged-content > staged-new.md && git add staged-new.md
echo v3-staged-edit > tracked.txt && git add tracked.txt
git worktree add -q ../wt2 -b feat-b main
check "T2 no staged new file" test ! -e ../wt2/staged-new.md
check "T2 committed tracked content" grep -q v2 ../wt2/tracked.txt
check "T2 clean destination" test -z "$(git -C ../wt2 status --porcelain)"

echo "=== T3 switching to main does not inject primary changes"
reset_fixture; dirty_live
git worktree add -q ../wt3 -b feat-c old
git -C ../wt3 switch -qc feat-d main
check "T3 committed tracked content" grep -q v2 ../wt3/tracked.txt
check "T3 no untracked copy" test ! -e ../wt3/newskill
check "T3 clean destination" test -z "$(git -C ../wt3 status --porcelain)"

echo "=== T4 the primary checkout is unchanged"
check "T4 tracked edit intact" grep -q v3-live-edit "$T/live/tracked.txt"
check "T4 primary status intact" test "$(git -C "$T/live" status --porcelain | wc -l | tr -d ' ')" = 2

echo "=== T5 a file checkout does not start the sandbox build"
reset_fixture
mkdir -p test
cp /bin/echo test/build
git add test/build && git commit -qm build
git worktree add -q ../wt5 -b feat-e main
for _ in $(seq 1 250); do
  [[ -e ../wt5/.install-test-home.log ]] && break
  sleep 0.02
done
check "T5 build precondition" test -e ../wt5/.install-test-home.log
rm -f ../wt5/.install-test-home.log
git -C ../wt5 checkout -q -- tracked.txt
is_build_absent=1
for _ in $(seq 1 50); do
  [[ ! -e ../wt5/.install-test-home.log ]] || { is_build_absent=0; break; }
  sleep 0.02
done
check "T5 no build" test "$is_build_absent" -eq 1
check "T5 committed content" grep -q v2 ../wt5/tracked.txt

echo "=== T6 a checkout starts the sandbox build"
reset_fixture
mkdir -p test
cp /bin/echo test/build
git add test/build && git commit -qm build
git worktree add -q ../wt6 -b feat-f main
for _ in $(seq 1 250); do
  [[ -e ../wt6/.install-test-home.log ]] && break
  sleep 0.02
done
check "T6 build ran" test -e ../wt6/.install-test-home.log
check "T6 clean destination" test -z "$(git -C ../wt6 status --porcelain)"

echo "=== T7 a no-argument call does not copy primary changes"
reset_fixture; dirty_live
git worktree add -q ../wt7 --no-checkout -b feat-g main
git -C ../wt7 checkout -q feat-g
( cd ../wt7 && ./hooks/post-checkout )
check "T7 no untracked copy" test ! -e ../wt7/newskill
check "T7 committed tracked content" grep -q v2 ../wt7/tracked.txt

echo "=== T8 agent authoring does not carry unrelated primary work"
reset_fixture
printf 'primary work\n' > unrelated.txt
primary_status="$(git status --porcelain)"
git worktree add -q ../wt8 -b feat-h main
mkdir -p ../wt8/agents/implementer/evals
printf 'definition\n' > ../wt8/agents/implementer/implementer.md
cp /usr/bin/true ../wt8/agents/implementer/evals/run
( cd ../wt8 && ./agents/implementer/evals/run )
check "T8 definition isolated" test -f ../wt8/agents/implementer/implementer.md
check "T8 evaluation isolated" test -x ../wt8/agents/implementer/evals/run
check "T8 no unrelated copy" test ! -e ../wt8/unrelated.txt
check "T8 no primary artifact" test ! -e agents/implementer
check "T8 primary unchanged" test "$(git status --porcelain)" = "$primary_status"

echo "=== T9 a primary invocation changes no checked-out content"
reset_fixture
mkdir -p test
cp /bin/echo test/build
git add test/build && git commit -qm build
./hooks/post-checkout
for _ in $(seq 1 250); do
  [[ -e .install-test-home.log ]] && break
  sleep 0.02
done
check "T9 build ran" test -e .install-test-home.log
check "T9 clean primary" test -z "$(git status --porcelain)"

echo
if [[ $FAILS -eq 0 ]]; then echo "ALL PASS"; else echo "$FAILS FAILURES"; exit 1; fi
