Source demos/_settings.tape

Output demos/verify.gif
Output demos/verify.mp4

Source demos/_cs_setup.tape
Type "clear"
Enter
Wait /\$/
Show

Type "# Build release files into a staging directory"
Enter
Sleep 500ms

Type@80ms `cs build --runtime-name demo --delegate-executable conda --artifact-layout online --install-name demo --out-dir "$DEMO_HOME/dist"`
Enter
Wait /\$/
Sleep 1s

Type "# Verify the staged files before publishing"
Enter
Sleep 500ms

Type@80ms `cd "$DEMO_HOME/dist"`
Enter
Wait /\$/
Sleep 1s

Type@80ms `ls -1`
Enter
Wait /\$/
Sleep 1s

Type@80ms `shasum -a 256 --check demo.sha256`
Enter
Wait /\$/
Sleep 1s

Type@80ms `grep -E '"(layout|binary|bundle|package_count)"' demo.info.json`
Enter
Wait /\$/
Sleep 3s
