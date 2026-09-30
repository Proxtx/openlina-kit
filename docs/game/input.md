# Input

Written against Steam build 22056877 (`hlboot.dat` sha256 `57afd61a…d01c`). Line numbers refer to the original Haxe
sources listed in the debug info; findexes (`fn@N`) refer to this build. Index: [../game-internals.md](../game-internals.md).

Default keyboard map (`KeymapManager.defaultKeymap`, closure fn@17238; SDL scancodes → actions):

| keys | action |
|---|---|
| arrows / WASD | move, aim |
| Z, Y, J, Space | jump |
| X, E, K | shoot (use item) |
| C, Q, L, Shift (+ codes 256/257, probably mouse buttons) | switch item |
| R | retry |
| Return / Esc | confirm / back |

Per frame, `PlayerInputs.updateSP` (called from `Main.mainLoop`) reads keyboard/gamepad into 9 action slots:
`inputs[i] = frame` while action `i` is held (`isDown(i)` = `inputs[i] == frame`). Bits for `toBin`/`readBin`
(replays): 0 up, 1 down, 2 left, 3 right, 4 jump, 5 shoot, 6 switch, 7 restart, 8 pause. `readBin(bits)` advances
the frame and sets the held actions, which is how the harness plays scripted inputs.

`xdotool` (XSendEvent) does not reach the game; use the harness.

- **Jump fires on the press, not while held**: holding `jump` from before Lina lands does not jump again; scripted
  inputs press it again (`"60:jump"`) once she stands (found by the moon-gravity mod, whose slow fall made Lina land
  later than a held jump started).
- **Aiming** (found by the swap mod, long aim, greendemo 1): the crosshair is a fixed offset from Lina per held
  direction, not a rotating reticle. Nothing held: forward, about 10° up (≈ (97, -16) from Lina); `up` (also
  `up+right`): about 76° up (≈ (24, -97)); `down`: nearly straight down. Releasing the key returns to forward, so
  scripted inputs must hold the direction on the shoot tick (`"36-52:up", "50:shoot"`).
