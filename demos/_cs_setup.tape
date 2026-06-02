Hide
Type `export PS1="$DEMO_PS1" && DEMO_HOME="$(mktemp -d /tmp/cs-demo-home.XXXXXX)" && export HOME="$DEMO_HOME"`
Enter
Wait /\$/
Type `eval "$(pixi shell-hook -s bash)"`
Enter
Wait /\$/
Type `export PS1="$DEMO_PS1" && cargo build --quiet --bin cs --bin cs-template`
Enter
Wait /\$/
Type `export PATH="$PWD/target/debug:$PATH" CONDA_SHIP_TEMPLATE="$PWD/target/debug/cs-template"`
Enter
Wait /\$/
