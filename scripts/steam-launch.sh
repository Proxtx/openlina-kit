#!/bin/sh
# Launch Mosa Lina with mods from Steam.
#
# Steam > Mosa Lina > Properties > Launch Options:
#     "/path/to/mosa-mod/scripts/steam-launch.sh" %command%
#
# Steam normally starts `Mosa Lina`, the HL/C native build, which cannot load modded
# bytecode. This script starts the HashLink JIT (`Mosa Lina_jit`) with work/hlboot.modded.dat
# instead. If no modded build exists it runs the original command (vanilla game).
# Set MOSA_VANILLA=1 to force vanilla.

REPO=$(cd "$(dirname "$0")/.." && pwd)
GAME="${MOSA_GAME_DIR:-$HOME/.local/share/Steam/steamapps/common/Mosa Lina}"
BYTECODE="$REPO/work/hlboot.modded.dat"

if [ -n "$MOSA_VANILLA" ] || [ ! -f "$BYTECODE" ] || [ ! -x "$GAME/Mosa Lina_jit" ]; then
    exec "$@"
fi

cd "$GAME" || exit 1
export LD_LIBRARY_PATH="$GAME${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
exec "./Mosa Lina_jit" "$BYTECODE"
