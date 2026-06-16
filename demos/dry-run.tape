Source demos/_settings.tape

Output demos/dry-run.gif
Output demos/dry-run.mp4

Source demos/_cs_setup.tape
Type "clear"
Enter
Wait /\$/
Show

Type "# Preview release artifacts without writing files"
Enter
Sleep 500ms

Type@80ms "cs build --dry-run --runtime-name demo --delegate-executable conda --artifact-layout online --install-name demo"
Enter
Wait /\$/
Sleep 3s

Type "# Change the layout when distribution needs change"
Enter
Sleep 500ms

Type@80ms "cs build --dry-run --runtime-name demo --delegate-executable conda --artifact-layout embedded --install-name demo"
Enter
Wait /\$/
Sleep 4s
