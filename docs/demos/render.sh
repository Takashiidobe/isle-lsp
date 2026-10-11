#!/bin/sh
# usage: docs/demos/render.sh <name>   (run from the repository root)
# needs vhs, ttyd, ffmpeg, node, chromium, and your init.lua with the isle commands
# a tape that opens no page (the xdg-open shim) is a terminal-only demo
# writes docs/images/<name>.gif; everything else lands in the ignored docs/demos/out
set -eu

name=${1:?demo name, e.g. trie}
here=$(cd "$(dirname "$0")" && pwd)
export DEMO_NAME=$name
export DEMO_OUT=$here/out
export PATH=$here/bin:$PATH

IMAGES=$here/../images

mkdir -p "$DEMO_OUT" "$IMAGES"
[ -d "$here/node_modules" ] || (cd "$here" && npm install --silent)

rm -f "$DEMO_OUT/$name.html"
node "$here/tape.js" "$here/$name.tape" "$DEMO_OUT/$name.rendered.tape"
vhs "$DEMO_OUT/$name.rendered.tape"

if [ -f "$DEMO_OUT/$name.html" ]; then
  node "$here/browser.js" "$name" "$DEMO_OUT/$name.html" "$DEMO_OUT/$name-browser.webm"

  fade=0.8
  offset=$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$DEMO_OUT/$name-terminal.webm")

  ffmpeg -y -loglevel error \
    -i "$DEMO_OUT/$name-terminal.webm" -i "$DEMO_OUT/$name-browser.webm" \
    -filter_complex "[0:v]scale=1400:800,setsar=1,fps=25,tpad=stop_mode=clone:stop_duration=$fade[a];[1:v]scale=1400:800,setsar=1,fps=25[b];[a][b]xfade=transition=fade:duration=$fade:offset=$offset" \
    -c:v libvpx-vp9 -b:v 0 -crf 34 "$DEMO_OUT/$name.webm"
else
  cp -f "$DEMO_OUT/$name-terminal.webm" "$DEMO_OUT/$name.webm"
fi

ffmpeg -y -loglevel error -i "$DEMO_OUT/$name.webm" \
  -vf "fps=12,scale=900:-1:flags=lanczos,split[s0][s1];[s0]palettegen[p];[s1][p]paletteuse" \
  "$IMAGES/$name.gif"

echo "wrote $IMAGES/$name.gif"
