#set document(title: [Text Fields on iOS Safari])
#set heading(numbering: "1.1")
#set ref(supplement: it => if it.func() == heading { "§" } else { it.supplement })
#show title: set align(center)
// Long tables run on across pages.
#show figure.where(kind: table): set block(breakable: true)

// Tables with rules above, below and under the header only.
#let rules(columns: auto, header, ..cells) = table(
  columns: columns,
  stroke: none,
  align: left,
  table.hline(),
  table.header(..header.map(strong)),
  table.hline(stroke: 0.5pt),
  ..cells,
  table.hline(),
)

// How a row of the map compares with a native field.
#let same = text(fill: rgb("#2e7d32"), weight: "bold")[same]
#let differs = text(fill: rgb("#c62828"), weight: "bold")[differs]
#let partly = text(fill: rgb("#c27c0e"), weight: "bold")[partly]

#title()

On iOS, a text field on a web page offers more than typing. You can double-tap a word to
select it, drag the handles, copy, cut and paste from the edit menu, and hold a finger down
to see a magnifier. Nabla's text fields are drawn by egui on a canvas, so they get none of
this from the browser. A first fix, in an egui fork, lays eframe's hidden `<input>` over the
focused field. That brings back the edit menu, but only with Paste in it. This document
records what a native field does on iOS Safari, compares Nabla's fields with it, explains
the gap, and sets out the properties a principled fix must have. The fix itself comes later,
built on the current code.

*In one sentence:* iOS's text interaction works on the DOM's text and selection, and
eframe's hidden input holds neither. A fix must keep that input an exact, invisible mirror of
the focused field, let the browser handle touches inside it, and report the results back to
egui.

#outline()

= Where things stand <current>

eframe's web backend has a *text agent*: a hidden `<input>` next to the canvas
(`crates/eframe/src/web/text_agent.rs`). While an egui `TextEdit` has focus, the agent has
the DOM focus, so the on-screen keyboard and the IME type into it. Its `input` and
`composition*` events become egui `Event::Text` and `Event::Ime`. Copy, cut and paste are
`ClipboardEvent`s on the document (`events.rs`): `paste` becomes `Event::Paste`, while
`copy` and `cut` run egui at once and write egui's `OutputCommand::CopyText` with
`navigator.clipboard.writeText`.

The agent's value is not the field's text. It holds only what the keyboard is composing, and
eframe clears it after anything else. egui gives the integration `IMEOutput`: the field's
`rect`, the `cursor_rect`, the `purpose` (`Normal`, `Password` or `Terminal`) and whether to
interrupt a composition. It gives neither the field's text nor its selection.

The fork (`vini-fda/egui`, branch `ios-paste`, commit `35182136a`; used by Nabla since
`c1cae39`) changes three things on a touch screen:

- The agent lies over the text of the focused field. It is opaque (`opacity: 1`), but its
  text, caret and background are transparent, and it takes touches (`pointer-events: auto`).
- Touches on the agent still reach egui as pointer events, so egui can place its cursor.
- eframe neither cancels those touches nor moves the focus away and back on them, so the
  browser can open its edit menu.

This is enough for Paste. Holding a finger on the field, or tapping its caret, opens iOS's
menu, and Paste inserts at egui's cursor. Everything else in @map is still missing.

= The map <map>

== How it was observed <method>

Observed on 6 November 2026 in the iOS 27.0 Simulator (iPhone 17), in Safari:

- *Native:* a test page with three `<input>`s: a URL with `autocorrect="off"` and
  `spellcheck="false"`, an empty field, and a password. The page logged `focus`, `blur`,
  `selectionchange` (with `selectionStart` and `selectionEnd`), `dblclick`, `copy`, `cut`,
  `paste`, `beforeinput` and `input`.
- *Nabla today:* a debug build of `ios-paste` (fork `35182136a`) served by trunk. The field
  tested was the server address under Online.

Two artifacts of the Simulator:

- Right after a page loads, the first tap does nothing, on native fields too. So this is not
  a fault of Nabla's.
- Two taps sent as separate commands sometimes arrive close enough together to count as a
  double tap.

Rows marked _device_ still need a check on a real iPhone.

== Gestures <gestures>

#figure(
  rules(
    columns: (0.75fr, 1.3fr, 1.3fr, auto),
    ([Gesture], [Native `<input>`], [Nabla today], []),
    [Tap an unfocused field],
    [Takes the focus and shows the keyboard. The caret goes to the *nearer word boundary*:
      a tap on the "m" of "example" put it after the word (`[15, 15]`), and a tap on the
      "o" of "some" put it before (`[25, 25]`).],
    [Takes the focus and shows the keyboard. egui puts its cursor at the *nearest character*.],
    partly,

    [Tap the caret, or the word it is in],
    [The caret stays put, and the edit menu toggles: *Paste | Select | Select All | AutoFill*.],
    [The menu opens, but with only *Paste | AutoFill*, the menu of an empty field.],
    differs,

    [Tap elsewhere in a focused field],
    [Moves the caret, to a word boundary, and closes the menu.],
    [Moves egui's cursor, to a character.],
    partly,

    [Double-tap a word (field focused)],
    [Selects the word (`dblclick`, then `selectionchange [25, 29]`) and shows *handles* at
      both ends and the menu: *Cut | Copy | Paste | AutoFill | ›*.],
    [egui selects the word in its own colour. There are no handles and no menu, and nothing
      to copy with.],
    differs,

    [Double-tap an unfocused field],
    [One run: the first tap focused the field and the second opened the caret's menu
      (_device_: may select the word on a phone).],
    [Focus, then egui selects the word.],
    partly,

    [Touch and hold on text],
    [A *magnifier* appears above the finger, with a caret that follows it. The menu opens on
      release (an empty field: *Paste | AutoFill*).],
    [egui collapses its selection to a cursor under the finger, and iOS opens *Paste |
      AutoFill*. There is no magnifier over egui's text.],
    differs,

    [Drag a selection handle],
    [The magnifier follows the handle, and the selection grows *by character*
      (`[30, 34]` → `[30, 37]` → `[30, 39]` → `[30, 40]`). The menu comes back on release.],
    [There are no handles.],
    differs,

    [Menu › (with a selection)],
    [A larger menu: Cut, Copy, Paste; AutoFill ›; Find; and, for a URL, Open Link and Add to
      Reading List.],
    [Not reachable.],
    differs,

    [Copy from the menu],
    [Fires `copy` with the selection `[30, 34]`, and the system clipboard then held "long"
      (`xcrun simctl pbpaste`). The menu closes and the selection stays.],
    [No Copy item.],
    differs,

    [Paste from the menu],
    [`paste`, then `beforeinput` and `input`, both `insertFromPaste`.],
    [eframe takes the document's `paste` as `Event::Paste` and inserts at egui's cursor.],
    same,

    [Tap outside],
    [`blur`. The keyboard goes away, and the field keeps its selection.],
    [egui loses the focus, and the keyboard goes away.],
    same,
  ),
  caption: [What each gesture does, on a native field and on Nabla's address field.],
) <fig-gestures>

== The edit menu <menu>

The menu's items follow from the DOM's state, and nothing else:

#figure(
  rules(
    columns: (auto, 1fr),
    ([State of the field], [Menu]),
    [Empty, caret], [Paste | AutoFill],
    [Text, collapsed caret], [Paste | Select | Select All | AutoFill],
    [Text, a selection], [Cut | Copy | Paste | AutoFill | › (or Replace… when spelling is checked)],
    [Password, caret], [_device_: no Copy or Cut for a password],
  ),
  caption: [The edit menu by state. The agent is always "empty", so it always shows the first row.],
) <fig-menu>

== Styling the input <styling>

eframe hides the agent with `color`, `caret-color` and `background-color` set to
`transparent`. A field styled like this, over a dark page, gave the following:

#figure(
  rules(
    columns: (1.4fr, auto, auto, 1fr),
    ([Style of the `<input>`], [Selects], [Menu], [Highlight and handles]),
    [Ordinary (opaque text)], [yes], [yes], [blue highlight, handles],
    [`color`, `caret-color`, `background` transparent (the agent today)],
      [yes], [yes], [only a faint dark box, *no handles*],
    [Text at alpha 0.02, `caret-color: transparent`], [yes], [yes], [only a faint dark box, *no handles*],
    [`color: transparent`, `caret-color` left alone], [yes], [yes], [*blue highlight and handles*, over invisible text, and a blue caret],
    [`opacity: 0` (found while writing the fork)], [–], [*no menu at all*], [–],
  ),
  caption: [What iOS draws for an input styled to be invisible.],
) <fig-styling>

*`caret-color: transparent` makes iOS hide the handles and the highlight. Transparent text
does not.* With transparent text and the default caret colour, iOS draws its whole selection
interface (caret, highlight, handles) over text you cannot see. That is what a mirror needs
(@properties).

== The keyboard <keyboard>

- *Suggestions.* Nabla's agent shows the QuickType bar ("I | The | I'm"), because it doesn't
  turn off `autocorrect`. The native URL field, which does, shows "Passwords" instead.
- *Capitals.* A field left alone starts with Shift on. The agent has `autocapitalize="off"`,
  like the URL field.
- *Passwords.* The password field's keyboard has no emoji key. A tap put the caret at the
  end (`[7, 7]`), and a double tap selected nothing.
- *Form bar.* Both show the bar above the keyboard, with up and down arrows and a check mark.
  The arrows move between the page's fields, and Nabla's page has only one, the agent.
- *Scrolling.* In both cases Safari scrolled the page to keep the focused field above the
  keyboard.

== Not observed here <unobserved>

These are part of iOS's text interaction, but this run did not try them. Check them on a
device:

- a triple tap
- dragging the caret itself
- Select and Select All, as a result rather than as menu items
- Cut
- holding the space bar to move the caret
- the three-finger gestures: pinch to copy, spread to paste, swipe to undo and redo
- shaking to undo
- dictation
- Look Up, Translate and Share for a selected word
- on an iPad: a hardware keyboard's shortcuts, and Scribble

Once the DOM's text and selection are right (P1), the browser provides most of these,
provided egui hears about what they change (P4).

= Why the agent cannot do this today <why>

+ *Its value is empty.* iOS reads words, selections, handles and menu items from the DOM's
  text, and the agent holds none of it. A double tap natively finds no word, and the menu
  offers only what fits an empty field (@fig-menu). This is what Nabla's "Paste | AutoFill"
  is.
+ *Its caret is transparent.* Even with the right text, `caret-color: transparent` hides the
  handles and the highlight (@fig-styling).
+ *A touch has two owners.* Since the fork, a touch on the agent goes to egui as a pointer
  event *and* to iOS as a text gesture, and each acts on its own. On a long press, egui
  collapsed its selection to a cursor while iOS opened its menu. On a double tap, egui
  selected a word that iOS knows nothing about.
+ *egui does not export the field.* The integration gets the field's `rect` and
  `cursor_rect` (`IMEOutput`), but not its text, its selection, its font or how far it is
  scrolled. eframe could not mirror the field even if it wanted to.
+ *Copy needs a secure context.* eframe answers `copy` and `cut` with
  `navigator.clipboard.writeText`, which is missing outside a secure context. Over
  `http://<LAN address>`, Copy would fail silently. The event's own `clipboardData` works
  anywhere. (Found by reading the code; not yet seen on a device.)

= Properties we want <properties>

== Notation

For the focused field: $T$ is egui's text, and $S = [a, b)$ is egui's selection, in
characters. $V$ is the agent's value, and $D = [s, e)$ is its DOM selection, in UTF-16 code
units. $u(i)$ converts a character index of $T$ to a UTF-16 index of $V$. $x_"egui" (i)$ is
where egui draws the caret at index $i$, and $x_"DOM" (i)$ is where the browser lays out
the agent's caret at $u(i)$, both in CSS pixels.

== The properties

#figure(
  rules(
    columns: (auto, 1fr, auto),
    ([], [Property], [Priority]),
    [P1], [*Mirror.* While a `TextEdit` has focus from a touch, after every frame,
      $V = T$ and $D = [u(a), u(b))$. A password is the exception (P8).], [must],
    [P2], [*Geometry.* $|x_"DOM" (i) - x_"egui" (i)| <= epsilon$ (about 1 px) for every
      visible $i$, and the agent's line box is egui's row. Then the handles, the
      magnifier, the menu and the browser's word hit-testing land on what egui draws. This
      takes egui's own font (its bytes, as a `FontFace`), its size times the zoom (not
      eframe's fixed 16 px), no ligatures, the same letter spacing and padding, and
      `scrollLeft` set to egui's horizontal scroll. E2 measured 0.17 px for Nabla's
      address field (@results).], [must],
    [P3], [*One owner per touch.* On a touch screen, touches inside the focused field belong
      to the browser: tap, double tap, hold and handles. egui hears only the outcome, as text
      and selection changes (P4). Touches elsewhere, and mouse input everywhere, stay
      egui's, as now.], [must],
    [P4], [*Changes come back.* Whatever changes $V$ or $D$ reaches egui and makes $T = V$
      and $S = D$ before the next frame: `input` of every type (typing, autocorrection,
      paste, cut, dictation, undo) and `selectionchange`. IME composition keeps working as
      it does now.], [must],
    [P4a], [*No echoes.* eframe writes $V$ or $D$ only when egui changed $T$ or $S$ for a
      reason of its own, never to hand back what the DOM just reported. It knows which is
      which by comparing with the last $V$ and $D$ it read, not with a flag: its own
      `selectionchange` arrives after it returns. It doesn't write while a finger is down on
      the agent. Every write closes the edit menu (@results).], [must],
    [P5], [*One caret, one selection.* Exactly one of each is visible. Since iOS's handles
      come only with its own highlight (@fig-styling), iOS draws the caret, the highlight
      and the handles while the agent covers the field, and egui doesn't. The agent's
      `caret-color` is egui's cursor colour, and iOS tints the caret, the highlight and the
      handles with it (E1).], [must],
    [P6], [*The menu fits the state.* As @fig-menu: Paste | Select | Select All for a caret,
      Cut | Copy | Paste for a selection. This follows from P1.], [must],
    [P7], [*Clipboard.* Copy and Cut put exactly the selected text on the clipboard,
      outside a secure context too, by the browser's own default action on the mirror or
      by `clipboardData`. Paste inserts at $D$.], [must],
    [P8], [*Passwords.* A password's text doesn't sit copyable in the DOM. The menu offers no
      Copy or Cut, and a double tap selects no words, as natively (@keyboard).], [must],
    [P9], [*Keyboard hints.* Each field says what it is for: `inputmode`, `autocorrect`,
      `spellcheck`, `autocapitalize`, `enterkeyhint`. For Nabla: a URL keyboard, no
      autocorrection and "Go" for the server address. This needs more from egui than
      today's `IMEPurpose`.], [should],
    [P10], [*Long text.* When a single-line field scrolls, egui and the agent scroll
      together, including when a handle is dragged to the edge.], [should],
    [P11], [*No regressions.* Desktop mouse and keyboard, Android Chrome, IME composition
      (with the Android and iOS Korean special cases in `text_agent.rs`), editing a
      `DragValue`, and multi-line `TextEdit`s (which would need a `<textarea>`).], [must],
  ),
  caption: [The properties a fix must have.],
) <fig-properties>

P1, P2 and P3 together are the principle: *iOS edits a real field, and that field looks
exactly like egui's.* The rest follows from them, or protects what already works.

= Directions for the fix <directions>

*A. Mirror the field in the agent (proposed).* egui would export the focused field's text,
selection, font, size and scroll to the integration, as more `IMEOutput` or as an output of
its own. It would also take two new inputs: set the selection, and replace a range of text.
eframe would keep the agent equal to the field (P1, P2), hand touches inside it to the browser
(P3), turn `input` and `selectionchange` into the new events (P4), and leave Copy to the
browser (P7). This is how other engines that draw their own text fields do it. Flutter's web
engine, for one, keeps a DOM input in sync with its editing state, placed and styled like the
field.

*B. Draw the handles, magnifier and menu in egui.* This would not look like iOS, and could
not paste without a separate step. A menu that egui draws cannot fire a trusted `paste`. It
would have to call `navigator.clipboard.readText`, which iOS confirms with a "Paste" bubble
of its own, and which needs a secure context. This is close to the separate Paste button
already turned down.

A makes the browser do the work and stays close to what eframe already is. Its risk was P2,
and E2 found it small (@results).

== Experiments to settle first <experiments>

#figure(
  rules(
    columns: (auto, 1fr, auto),
    ([], [Question], [Status]),
    [E1], [Do the highlight and the handles take the agent's `caret-color`, so they match
      egui's selection colour?], [yes (@results)],
    [E2], [How closely does the browser lay out egui's font, loaded as a `FontFace`? Measure
      $x_"DOM"$ (with `Range.getBoundingClientRect`) against $x_"egui"$ for each glyph.],
      [0.17 px (@results)],
    [E3], [Can `setSelectionRange`, called while the menu or a handle drag is active, leave
      them alone? It must not echo P4's own changes back.], [no (@results)],
    [E4], [Passwords: `type="password"` on the agent only while it covers a field (eframe
      avoids it because of Chrome's password manager), or no mirror at all?], [open],
    [E5], [On a phone, does a double tap on an unfocused field select the word?], [open],
    [E6], [Does Copy fail over `http://<LAN address>` today, as the code suggests?], [open],
    [E7], [Does a write during a *handle* drag end the drag? (E3 caught only a caret
      drag.)], [open],
    [E8], [Without `user-scalable=no`, does an agent smaller than 16 px zoom the page on
      focus, and does a 16 px agent scaled down with a `transform` keep P2?], [open],
  ),
  caption: [What to find out before building.],
) <fig-experiments>

= Results of E1–E3 <results>

Run on 6 November 2026, in the same Simulator as @method (iOS 27.0, iPhone 17,
`devicePixelRatio` 3). Each experiment was a plain page of `<input>`s whose viewport, like
Nabla's, has `user-scalable=no`.

== E1: the selection's colour

The test field had transparent text and `caret-color: #cba6f7`, the lavender of Nabla's
focused field.

- *The caret* was lavender.
- *A double tap* selected the word. The handles were lavender and the highlight was a
  translucent lavender, both over the invisible text.
- *The menu* (Cut | Copy | Paste | Replace… | ›) was as usual.

So one CSS property gives iOS's whole selection interface egui's colour (P5).

== E2: where the browser puts the caret

The fonts were the ones Nabla embeds (`crates/nabla-client/assets/fonts`), loaded with
`@font-face` from the same files. epaint (fork `35182136a`) laid out two strings at 3 pixels
per point, and gave $x_"egui" (i)$ for every index $i$ (`Galley::pos_from_cursor`). The
page measured $x_"DOM" (i)$ with a `Range` over the first $i$ characters. It measured each
string twice: at the field's size, and at 100 times that size, divided by 100, because WebKit
rounds a range's rectangle to the pixel grid.

#figure(
  rules(
    columns: (1.5fr, 1fr, 1fr, auto),
    ([String (field)], [Font], [Max $|x_"DOM" - x_"egui"|$], [At the end]),
    [`https://localhost:4433/` `#sha256=ab12->x` (server address; measured with a LAN IP in place of `localhost`)],
      [Lilex 12.5 px], [0.17 px (rms 0.12)], [307.50 = 307.50],
    [`AVATAR Toy Wave -15.00 kg m^2 office` (labels, password)],
      [IBM Plex Sans 13.5 px], [0.29 px, but 3.95 px inside "ffi"], [238.00 = 238.00],
  ),
  caption: [The browser's caret positions against egui's, in CSS pixels.],
) <fig-e2>

- *The error is egui's rounding to the pixel grid.* egui snaps each glyph to a physical
  pixel (1/3 px here). In Lilex, every other advance is 7.33 px in egui's layout and 7.50 px
  in the browser's. The error never builds up: both strings end at the same $x$.
- *Kerning matches.* Plex's "AV", "AT" and "Wa" agree within 0.29 px, so the browser and
  egui shape the same way.
- *Ligatures do not.* Inside the "ffi" ligature the two disagree by up to 4 px: egui puts
  carets at 224.00 then 223.67, so its caret even moves backwards, while the browser spreads
  them across the glyph. Turning ligatures off in the agent (`font-variant-ligatures: none`)
  removes this, if egui keeps drawing them. It doesn't affect Nabla's address field.
- *No zoom.* A 12.5 px input took the focus without the page zooming in, under
  `user-scalable=no`. eframe's fixed 16 px, there to prevent that zoom, isn't needed in that
  case, and the agent can be egui's size (P2). Without `user-scalable=no` this is
  untested (E8).

P2 holds with a margin of about 6× for the address field, and for proportional text outside
ligatures.

== E3: writing to the input while iOS is using it

#figure(
  rules(
    columns: (auto, 1fr, 1fr),
    ([Case], [What the page did], [What iOS did]),
    [E3p],
    [After a double tap had selected "push" and the menu was open, it called
      `setSelectionRange(0, 3)`.],
    [The highlight and the handles moved to "E3p", and *the menu closed*. The
      `selectionchange` for the call came *after* it returned.],

    [E3v],
    [After a double tap had selected "change" and the menu was open, it put an "X" in front
      of the value and moved the selection one place on.],
    [The selection stayed on "change" and the handles moved with it, and *the menu
      closed*.],

    [E3v, mid-gesture],
    [It made the same change while a finger was dragging the caret, with the magnifier up.
      The touch had missed the handle, so this was a caret drag.],
    [The magnifier kept following the finger, but no further `selectionchange` came. After
      the finger lifted, the caret was where the page had put it, *not under the finger*.],

    [E3s],
    [On every `selectionchange`, it wrote the selection it had just read back, 30 ms later.
      A flag set around the write was meant to mark the page's own changes.],
    [A double tap selected "every" ([18, 23]). The stale echo of the tap before it ([23,
      23]) undid that, iOS re-applied it, and so on: eight swaps in about a second. Then it
      settled on the stale caret. *The user's selection was lost and no menu opened.* The
      flag never matched, because the event comes after the write.],
  ),
  caption: [Writes to the input during iOS's text interaction.],
) <fig-e3>

So no, a write doesn't leave iOS alone. It closes the menu, and a write during a gesture beats
the gesture. eframe must write only when egui has a change of its own to make, compare
against the last value and selection it read rather than trust a flag, and wait until no
finger is down on the agent. That is P4a. A handle drag interrupted by a write is untested
(E7).

= Test plan <tests>

Each row of @fig-gestures and @fig-menu, and each item of @unobserved, becomes a check, run
on the Simulator and on an iPhone. Each check compares Nabla's address and password fields
with the native test page. A row passes when Nabla shows the same items, the same caret
positions and the same selections in characters. Desktop Chromium, desktop Safari and
Android Chrome are checked for P11.
