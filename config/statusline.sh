#!/bin/sh
# Claude Code renders this on one dim line and truncates on display width. Padding is
# therefore measured on the ANSI-free copy of each line, and colours close with [39m
# rather than [0m so the reset does not strip the surrounding dim style.
set -eu

# Claude Code exports COLUMNS from its own stdout width. Bash sets the variable to 0
# when no terminal is attached, which would drive every width budget negative.
cols=${COLUMNS:-80}
case "$cols" in '' | *[!0-9]*) cols=80 ;; esac
if [ "$cols" -lt 20 ]; then cols=80; fi

out=$(jq -r --argjson now "$(date +%s)" --argjson cols "$cols" '
  (.model.display_name // .model.id // "claude") as $m
| (.effort.level // "") as $e
| (if $e != "" then $m + " (" + $e + ")" else $m end) as $modelstr

| (.rate_limits.five_hour.used_percentage // null) as $five
| (.rate_limits.seven_day.used_percentage // null) as $week
| {p:$five, l:"5h", r:(.rate_limits.five_hour.resets_at // null), w:18000} as $session
| {p:$week, l:"7d", r:(.rate_limits.seven_day.resets_at // null), w:604800} as $weekly

# The weekly window is the standing view. The session window replaces it only once it is
# both near its cap and the worse of the two, so the line reports one number, not two.
| (if $five == null and $week == null then null
   elif $week == null then (if $five > 90 then $session else null end)
   elif $five == null then $weekly
   elif ($five > 90 and $five > $week) then $session
   else $weekly end) as $u

| (($cols * 25 / 100 | floor) as $b | if $b < 6 then 6 elif $b > 60 then 60 else $b end) as $barw

| (if $u == null then null else
     ($u.p | floor) as $p
   | (($u.r // $now) - $now) as $diff

   # On-pace is the share of the window already spent, read from its own reset stamp.
   # A calendar day index cannot serve here: neither window is pinned to a week boundary.
   | ((($u.w - $diff) / $u.w) * 100 | if . < 0 then 0 elif . > 100 then 100 else . end) as $pace
   | ($p * $barw / 100 | floor) as $fill
   | (($pace * $barw / 100 | floor) | if . < 0 then 0 elif . > ($barw - 1) then ($barw - 1) else . end) as $mark

   | ([range(0; $barw) | if . < $fill then "█" else "░" end] | join("")) as $barplain
   | ([range(0; $barw) | if . == $mark then "\u001b[93m│\u001b[39m" elif . < $fill then "█" else "░" end] | join("")) as $barcolor

   | (if $diff <= 0 then "<1m"
      elif ($diff / 86400 | floor) >= 1 then (($diff / 86400 | floor) | tostring) + "d " + (($diff % 86400 / 3600 | floor) | tostring) + "h"
      elif ($diff / 3600 | floor) >= 1 then (($diff / 3600 | floor) | tostring) + "h " + (($diff % 3600 / 60 | floor) | tostring) + "m"
      else (($diff / 60 | floor) | tostring) + "m" end) as $reset

   | (if (($five // 0) >= 100 or ($week // 0) >= 100) then "\u001b[91m"
      elif $p > $pace then "\u001b[93m"
      else "" end) as $numcolor
   | (if $numcolor == "" then "" else "\u001b[39m" end) as $numclose

   | {plain: ($u.l + " " + $barplain + " " + ($p | tostring) + "% (resets in " + $reset + ")"),
      color: ($u.l + " " + $barcolor + " " + $numcolor + ($p | tostring) + "%" + $numclose + " (resets in " + $reset + ")")}
   end) as $usage

| ($cols - 4) as $avail
| def ralign($plain; $colored):
    (if ($avail - ($plain | length)) > 0 then (" " * ($avail - ($plain | length))) else "" end) + $colored;

  if $usage == null then ralign($modelstr; $modelstr)
  else ($modelstr + " │ " + $usage.plain) as $onelineplain
     | ($modelstr + " │ " + $usage.color) as $onelinecolor
     | if ($onelineplain | length) <= $avail then ralign($onelineplain; $onelinecolor)
       else ralign($modelstr; $modelstr) + "\n" + ralign($usage.plain; $usage.color) end
  end
')

# Claude Code paints a whitespace-only line as a blank status line, but keeps the
# previous text when the command exits non-zero on empty stdout. Blank is the worse of
# the two, so an all-space render becomes a no-update instead.
case "$(printf '%s' "$out" | tr -d ' \n')" in
  '') exit 1 ;;
esac
printf '%s\n' "$out"
