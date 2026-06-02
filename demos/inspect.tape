Source demos/_settings.tape

Output demos/inspect.gif
Output demos/inspect.mp4

Source demos/_cs_setup.tape
Type "clear"
Enter
Wait /\$/
Show

Type "# Inspect the source environment before shipping it"
Enter
Sleep 500ms

Type@80ms "cs inspect"
Enter
Wait /\$/
Sleep 3s

Type "# CI can use the same preflight in JSON form"
Enter
Sleep 500ms

Type@80ms "cs inspect --json | python -m json.tool | sed -n '1,35p'"
Enter
Wait /\$/
Sleep 4s
