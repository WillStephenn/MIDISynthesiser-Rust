# MIDI Synthesiser Rust Port

This is a Rust port of the polyphonic MIDI synthesiser originally written in Java.

The original project can be found at https://github.com/WillStephenn/MIDISynthesiser.

This Rust port was initially ported by claude fable 5 to bring the synthesiser into a more appropriate language.

## Performance Comparison

Migrating the synthesiser from Java to Rust has resulted in reduced the total processing time by ~45%.

Test bench results:

| Language | Total Processing Time |
|---|---|
| Java | 93 ms |
| Rust | 51 ms |
