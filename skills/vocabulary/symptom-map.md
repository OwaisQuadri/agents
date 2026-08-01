# Symptom map

Ramble words on the left, candidate terms on the right. Every indexed term appears in
at least one family, so this also reads as an adjacency view: terms that fix the same
felt problem sit together. Still not an answer key — read each candidate's definition
in `vocabulary.md` and let the definition decide. Terms marked † are index-only — no
bundled definition, so SKILL.md's web-search rule applies. Grow the headings from
usage-log rambles the map missed.

## Cramped · tight · smushed · can't breathe · suffocating
Leading · Tracking · Gap · Negative space · Line length · Breathing room†

## Sparse · empty · floaty · disconnected · unfinished
Negative space · Gap · Grid · Hierarchy · Empty state

## Flat · washed out · bland → pop · punchy
Contrast ratio · Weight · Saturation · Chroma · Type scale · Hierarchy

## Cheap · generic · template-y · unpolished → premium · classy · refined
Tinted neutral · Type scale · Tracking · Negative space · Border radius · Visual language · Alpha

## Off · crooked · misaligned · something's wrong
Optical centre · Baseline grid · Grid · Optical kerning · Asymmetry · Z-index

## Janky · stuttery · choppy motion or scroll
Transition property · GPU compositing† · Easing · Duration · Layout shift

## Busy · cluttered · noisy · overwhelming
Negative space · Hierarchy · Progressive disclosure · Unified weight† · Visual language · Accordion · Tabs

## Jumpy · shifty · stuff moves on load · wiggling numbers
Layout shift · Font stack · Aspect ratio · Skeleton · Tabular nums · Variable font · Cap height · x-height

## Abrupt · harsh · robotic motion → smooth · natural · springy
Easing · Ease-out · Ease-in · Ease-in-out† · Duration · Stagger · Spring† · Choreography†

## Slow · sluggish · laggy · unresponsive-feeling
Duration · Optimistic update · Skeleton · Skeleton shimmer · Spinner · Debounce · GPU compositing†

## Hard to read · dense · tiring · wall of text
Line length · Leading · Type scale · Contrast ratio · x-height · Scannability†

## Lost · hard to find · confusing · where am I
Navigation · Hierarchy · Mental model · Wayfinding† · Signpost† · Breadcrumb · Search as escape hatch† · Labelling†

## Hard to hit · doesn't feel clickable · dead-feeling controls
Touch target · Affordance · Hover state · Active state · Cursor · Disabled state

## Broken-looking text · weird gaps · dangling word · cut off
Kerning · Optical kerning · Widow · Orphan · Text overflow · Hyphenation · Ligature · Truncation strategy†

## Muddy colors · dull · grey in the middle · lifeless tints
Gradient · OKLCH · Chroma · Tinted neutral · Blending · P3 · sRGB

## Dark mode looks wrong · glowing · vibrating colors
Dark mode · Saturation · Chroma · Contrast ratio · Prefers color scheme

## Alive · reactive · it should feel like it responds
Motion as feedback† · Hover state · Optimistic update · Spring† · Stagger

## Blurry · fuzzy · pixelated
Pixel hinting · HiDPI / Retina · Font smoothing

## Broken on my phone · cut off on mobile · doesn't fit · scrolls sideways
Responsive · Breakpoint · Overflow · Viewport units · Safe area · Max-width · Clamp

## Wordy · unhelpful error · robotic wording · doesn't tell me what to do
Error message · Microcopy · Front-loading · Inline error · CTA · Tone† · Error state · Voice†

## Inconsistent · mismatched · doesn't feel like one product
Design system · Tokens · Semantic token · Visual language · Icon library · Type scale · Source of truth · Variables

## One line taller than the rest · footnote breaks the spacing · the little numbers look huge
Superscript · Subscript · Leading

## Won't line up · spacing between items is uneven · I keep positioning things by hand
Flexbox · Auto layout · Gap · Grid

## Keep it on screen while scrolling · header scrolls away · pin it while the page moves
Sticky positioning · Overflow · Z-index

## Clicking does nothing · something invisible eats the click · clicking the label doesn't work · I hid it but it still takes up space
Pointer events · Opacity vs visibility · Z-index · Disabled state · Label association · Semantic HTML

## Icons look mismatched · some icons look heavier · like they came from different sets
Stroke weight · Filled vs outlined · Cap style · Icon size system† · Icon library · Unified weight† · Breathing room†

## What does this icon even mean · people click the wrong one · had to guess
Metaphor accuracy† · Meaning collision† · Contextual swap† · Labelling† · Tooltip

## Screens just swap · teleporting · no sense of moving through the app
Shared axis transition† · Enter vs exit asymmetry† · Choreography† · Stagger

## Makes me dizzy · too much movement · animations are exhausting → calmer
Reduced motion · Duration · Choreography† · Skeleton shimmer

## Tab jumps around · can't see where I am · tabbing escapes the popup · endless tabbing to get anywhere
Focus state · Tab order · DOM order · Skip link · Focus trap · Semantic HTML

## Does it work for blind users · it just says button · reads out the wrong order
Screen reader · aria-label · Semantic HTML · DOM order · Label association

## Too faint · can everyone see this · fails the accessibility check · colorblind users can't tell
Contrast ratio · WCAG · APCA · Color-only state

## Did it work · did it save · did that actually copy · no confirmation · people click it twice
Toast · Success message† · Optimistic update · Copy to clipboard · Active state · Error state · Motion as feedback†

## Scared to click · deleted it by accident · that button feels dangerous
Confirmation dialog · Destructive language† · Modal / Dialog · CTA

## New users don't get it · don't know where to start
Onboarding · Empty state · Contextual help† · Mental model · Progressive disclosure

## What goes in this box · people fill the form wrong · confusing fields
Placeholder · Label association · Inline error · Contextual help† · Microcopy

## Nothing's where they expect · categories feel wrong · buried too many clicks deep
Card sorting† · Content inventory† · Labelling† · Depth† · Mental model · Navigation

## Numbers look messy · don't line up · hard to compare · too many decimals
Numeric formatting† · Tabular nums · Data table

## Stiff · corporate · cold · impersonal · reads like a legal document → warm · friendly · human
Voice† · Tone† · Microcopy · Sentence case · Avatar

## Every button looks important · which one is the main action
Button · CTA · Hierarchy

## I need a text box · somewhere to type · let them write something
Input · Textarea · Placeholder · Label association

## I need them to pick one · pick all that apply · dropdown's too long · can't find the option
Select · Radio group · Checkbox · Combobox

## Turn it on or off · flip a setting · dial in a value
Switch · Checkbox · Slider

## I need a popup · make them confirm first · show it without leaving the page
Modal / Dialog · Confirmation dialog · Sheet · Drawer · Popover · Focus trap

## Needs explaining · a little hint on hover
Tooltip · Popover · aria-label · Contextual help† · Microcopy

## Show a count · mark it as new · status at a glance
Badge · Tag · Color-only state

## What kind of thing is this · needs a label · filter by type
Tag · Badge · Labelling† · Tabs

## Need a menu · how do people get around · too many sections to link
Navigation menu · Sidebar · Tabs · Breadcrumb · Command menu · Navigation

## Too many items · list never ends · can't fit them all on screen
Pagination · Carousel · Overflow

## Runs together · can't tell what belongs together · one big blob
Card · Separator · Gap · Negative space · Grid

## Is it stuck · how much longer · how many steps left
Progress · Stepper · Spinner

## Too flat · want it tactile · like a real object
Skeuomorphism · Blending · Visual language

## Build doesn't match the mockup · devs guessed the spacing · design and code drifted
Handoff · Redline / annotation · Source of truth · Tokens · Design system

## Design file is a mess · which mockup is current · screens all over the canvas
Artboard / Frame · Source of truth · Auto layout

## Can't explain it in words · need something to show before we build · we haven't agreed on a direction
Prototype · Moodboard

## Link looks bad when shared · no preview image · title gets cut off in Slack
Open Graph · Front-loading · Truncation strategy†

## Are people actually using this · what do users actually do · where do they get stuck
Heatmap · Session recording · Scroll depth · Funnel

## Nobody signs up · traffic but no action · people leave without doing anything
Conversion · Funnel · Bounce rate · CTA · A/B test

## People try it once and never come back · users keep cancelling · do they even like it
Retention · Churn · NPS · Onboarding
