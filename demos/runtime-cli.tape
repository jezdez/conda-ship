Source demos/_settings.tape

Output demos/runtime-cli.gif

Source demos/_cs_setup.tape
Hide
Type `cs build --runtime-name demo --delegate-executable conda --artifact-layout online --install-name demo --out-dir "$DEMO_HOME/runtime"`
Enter
Wait /\$/
Type "clear"
Enter
Wait /\$/
Show

Type "# Run the generated runtime binary"
Enter
Sleep 500ms

Type@80ms `DEMO_PREFIX="$DEMO_HOME/runtime-prefix" "$DEMO_HOME/runtime/demo" --version`
Enter
Wait /\$/
Sleep 1s

Type "# Later invocations go directly to the configured delegate"
Enter
Sleep 500ms

Type@80ms `DEMO_PREFIX="$DEMO_HOME/runtime-prefix" "$DEMO_HOME/runtime/demo" info --base`
Enter
Wait /\$/
Sleep 5s
