# VERITAS Healthcare Demo Recording
# Prerequisites: cargo build -p demo
# Run: vhs assets/demo.tape

Output assets/demo.gif
Set Theme "Catppuccin Mocha"
Set FontSize 13
Set Width 1200
Set Height 700
Set Padding 10
Set TypingSpeed 40ms
Set PlaybackSpeed 0.5

# Run individual scenarios for better pacing
Type "RUST_LOG=off ./target/debug/demo drug-interaction"
Enter
Sleep 5s

Type "RUST_LOG=off ./target/debug/demo note-summarizer"
Enter
Sleep 5s

Type "RUST_LOG=off ./target/debug/demo patient-query"
Enter
Sleep 8s
