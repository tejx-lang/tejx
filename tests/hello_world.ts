let x: number = 20;
let y: number = x + 22;
// console.log(y); // Need to verify if console.log is supported in new pipeline.
// For now, y is stored in a variable, but to see output we might need main to return it or printf.
// The current MIRCodeGen emits printf specific code for console.log?
// Wait, default MIRCodeGen I wrote didn't explicitly check for console.log CallExpr in `genInstruction`.
// It only had `Call` instruction but I didn't see special handling in `genInstruction`.
// Let's check `MIRLowering` if it lowers `console.log` specially.
// Steps 280/281: `lowerExpression` handles `HIRNewExpr`, `HIRVariable`, `HIRLiteral`, `HIRBinaryExpr`.
// It does NOT handle `CallExpr` yet!
// So `console.log` won't work.
// I should add `CallExpr` support to `Lowering` -> `HIR` -> `MIR` -> `CodeGen` if I want to see output.
// Or, I can check the return code?
// `main` returns 0.
// Let's add support for `return`.
return y; // This should make main return 42.
