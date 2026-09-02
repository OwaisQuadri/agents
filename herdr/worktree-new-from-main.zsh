#!/bin/zsh
# worktree-new-from-main — prefix+shift+g popup: create a new Git worktree
# based on the freshest origin/main, from any worktree checkout of the repo
# (no need to cd to the main checkout first). Fetches origin/main, then
# offers a random Name of Allah (with its English translation) as the
# branch name through the same fzf-list popup style as worktree-picker.zsh
# — arrow up/down browses all 99 names and translations, Enter accepts the
# highlighted one, typing searches the full list. Hands off to `herdr
# worktree create` with an explicit --cwd/--base so the new branch never
# inherits whatever HEAD the calling checkout happens to have (herdr's own
# --base default is HEAD, not main).
set -euo pipefail
# popups launch with a minimal PATH; herdr, git and fzf live in /opt/homebrew/bin
path=(/opt/homebrew/bin /usr/local/bin $path)

# Slug|Translation, one of the 99 Names of Allah (Asma al-Husna) per line.
# Source: surahquran.com/99-Names-of-Allah-en.html. A few transliterations
# that source repeats verbatim for two distinct Arabic names (e.g. Al-Malik
# used for both المالك and المليك) are disambiguated here so every slug in
# this list is unique.
names=(
  "Allah|The One True God"
  "Ar-Rahman|The Most Gracious"
  "Ar-Raheem|The Most Merciful"
  "Al-Malik|The Sovereign Lord"
  "Al-Quddus|The Most Holy"
  "As-Salam|The Source of Peace"
  "Al-Mumin|The Giver of Security"
  "Al-Muhaymin|The Guardian"
  "Al-Aziz|The Almighty"
  "Al-Jabbar|The Compeller"
  "Al-Mutakabbir|The Supremely Great"
  "Al-Khaliq|The Creator"
  "Al-Bari|The Originator"
  "Al-Musawwir|The Fashioner"
  "Al-Awwal|The First"
  "Al-Aakhir|The Last"
  "Az-Zahir|The Manifest"
  "Al-Batin|The Hidden"
  "Al-Maalik|The Owner"
  "Al-Maleek|The King"
  "Al-Hadi|The Guide"
  "As-Samee|The All-Hearing"
  "Al-Basir|The All-Seeing"
  "Al-Wasi|The All-Encompassing"
  "Al-Muhit|The Encompasser"
  "Allam-Al-Ghuyub|The Knower of the Unseen"
  "Ash-Shaakir|The Appreciative"
  "Al-Barr|The Source of Goodness"
  "At-Tawwab|The Acceptor of Repentance"
  "Ar-Rauf|The Most Kind"
  "Al-Wali|The Protecting Friend"
  "Al-Mawla|The Master"
  "Ar-Rabb|The Lord"
  "Al-Khallaq|The Supreme Creator"
  "Al-Qadeer|The All-Powerful"
  "An-Naseer|The Helper"
  "Al-Ghani|The Self-Sufficient"
  "Al-Hameed|The Praiseworthy"
  "Al-Majid|The All-Glorious"
  "Al-Haqq|The Truth"
  "Al-Mubeen|The Clear"
  "Al-Qawiyy|The All-Strong"
  "Al-Matin|The Firm"
  "Al-Muntaqim|The Avenger"
  "Al-Afuw|The Pardoner"
  "Al-Ghafur|The Oft-Forgiving"
  "Al-Haleem|The Forbearing"
  "Al-Qareeb|The Near"
  "Al-Mujib|The Responsive"
  "Al-Hayy|The Ever-Living"
  "Al-Qayyum|The Self-Subsisting"
  "Al-Aliyy|The Most High"
  "Al-Azim|The Magnificent"
  "Al-Kabeer|The Most Great"
  "Al-Mutaali|The Supreme"
  "Al-Latif|The Subtle"
  "Al-Khabir|The All-Aware"
  "Al-Wahhab|The Bestower"
  "Ar-Razzaq|The All-Provider"
  "Al-Haseeb|The Reckoner"
  "Ar-Raqeeb|The All-Watchful"
  "Ash-Shaheed|The All-Witnessing"
  "Al-Muqeet|The Sustainer"
  "Al-Fattah|The Opener"
  "Al-Aleem|The All-Knowing"
  "Al-Hakeem|The All-Wise"
  "Al-Jami|The Gatherer"
  "Al-Qadir|The Capable"
  "Al-Muqtadir|The Determiner"
  "Fatir-As-Samawati-Wal-Ard|The Originator of the Heavens and Earth"
  "Alim-Al-Ghayb-Wa-Ash-Shahadah|The Knower of Unseen and Seen"
  "Badi-Us-Samawati-Wal-Ard|The Incomparable Originator"
  "Nur-As-Samawati-Wal-Ard|The Light of the Heavens and Earth"
  "Al-Wahid|The One"
  "Al-Ahad|The Unique"
  "As-Samad|The Eternal Refuge"
  "Al-Qahir|The Subduer"
  "Al-Qahhar|The All-Subduer"
  "Al-Aalim|The Knower"
  "Al-Hakam|The Judge"
  "Al-Ilah|The Deity"
  "Al-Hafiyy|The Ever Gracious"
  "Al-Wadud|The Loving One"
  "Al-Hafiz|The Preserver"
  "Al-Hafeez|The Guardian"
  "Al-Ghalib|The Victorious"
  "Al-Kafi|The Sufficient"
  "Al-Mannan|The Bestower of Favors"
  "Al-Mustaan|The Source of Help"
  "Al-Warith|The Inheritor"
  "Al-Kafeel|The Guarantor"
  "Al-Wakeel|The Trustee"
  "Al-Ghaffar|The Repeatedly Forgiving"
  "Al-Karim|The Most Generous"
  "Ash-Shakur|The Most Appreciative"
  "Al-Ala|The Most Exalted"
  "Al-Akram|The Most Noble"
  "Malik-Ul-Mulk|The Owner of Sovereignty"
  "Dhul-Jalali-Wal-Ikram|The Lord of Majesty and Bounty"
)

repo_root="$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null)" \
  || { print "not inside a git repo" >&2; exit 1 }
repo_root="${repo_root:h}"

print "fetching origin/main in ${repo_root:t}..."
git -C "$repo_root" fetch origin main --quiet
print "fetched."

# put a random entry first so it's the default highlighted row; the rest
# stay in list order below it for browsing/searching (and for learning —
# every scroll shows another Name of Allah and its translation). Rows are
# "<display>\t<slug>\t<translation>", same tab-delimited shape as
# worktree-picker.zsh's rows.
default_idx=$(( RANDOM % ${#names[@]} + 1 ))
before_len=$(( default_idx - 1 ))
ordered=("${names[$default_idx]}" "${names[@]:0:$before_len}" "${names[@]:$default_idx}")
rows=""
for entry in "${ordered[@]}"; do
  slug_part="${entry%%|*}"
  translation_part="${entry##*|}"
  rows+="${slug_part} — ${translation_part}"$'\t'"${slug_part}"$'\t'"${translation_part}"$'\n'
done

pick="$(print -rn -- "$rows" |
  fzf --with-nth=1 --delimiter=$'\t' --prompt='> ' --height=100% --no-sort \
      --header='new worktree off freshest origin/main — enter: accept, type: search all 99 names')" \
  || { print "cancelled" >&2; exit 0 }
[[ -n "$pick" ]] || { print "cancelled" >&2; exit 0 }

parts=("${(@s:	:)pick}")
slug="$parts[2]"
translation="$parts[3]"

candidate="$slug"
n=0
while git -C "$repo_root" rev-parse --verify --quiet "refs/heads/$candidate" >/dev/null \
   || git -C "$repo_root" rev-parse --verify --quiet "refs/remotes/origin/$candidate" >/dev/null; do
  (( n += 1 ))
  candidate="${slug}_${n}"
done

print "creating worktree on branch ${candidate} (${translation})"
herdr worktree create --cwd "$repo_root" --branch "$candidate" --base origin/main --focus
herdr notification show "$candidate" --body "$translation" --sound done >/dev/null
