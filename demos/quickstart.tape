Source demos/_settings.tape

Output demos/quickstart.gif

Source demos/_cs_setup.tape
Hide
Type `mkdir -p "$DEMO_HOME/quickstart" && cp pixi.toml pixi.lock "$DEMO_HOME/quickstart/"`
Enter
Wait /\$/
Type `awk '{ print; if ($0 == "[tool.conda-ship]") { print "runtime-name = \"demo\""; print "delegate-executable = \"conda\""; print "artifact-layout = \"online\"" } }' "$DEMO_HOME/quickstart/pixi.toml" > "$DEMO_HOME/quickstart/pixi.toml.new" && mv "$DEMO_HOME/quickstart/pixi.toml.new" "$DEMO_HOME/quickstart/pixi.toml" && cd "$DEMO_HOME/quickstart"`
Enter
Wait /\$/
Type "clear"
Enter
Wait /\$/
Show

Type "# Quickstart: inspect packages, preview, build"
Enter
Sleep 500ms

Type@80ms "cs inspect | grep -E '^(Packages:|  (conda|conda-rattler-solver|conda-spawn|python) )'"
Enter
Sleep 2s

Type@80ms "cs build --dry-run | sed -n '1,32p'"
Enter
Sleep 3s

Type@80ms "cs build --out-dir dist"
Enter
Sleep 3s

Type@80ms "./dist/demo --version"
Enter
Sleep 3s
