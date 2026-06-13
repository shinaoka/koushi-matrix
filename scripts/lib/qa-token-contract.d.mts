// Type declarations for the JS QA token/private-data contract helper so
// TypeScript test files can import it without an implicit-any error.
export function tokensFromOutput(output: string): Set<string>;
export function assertRequiredTokens(
  output: string,
  requiredTokens: string[],
  label: string
): void;
export function assertNoMatrixIdentifiers(output: string, label: string): void;
