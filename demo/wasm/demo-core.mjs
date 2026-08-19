export const MAX_NAT = (1n << 64n) - 1n;

export function parseNat(value, label) {
  const normalized = String(value).trim();
  if (!/^[0-9]+$/.test(normalized)) {
    throw new Error(`${label} must be a non-negative whole number`);
  }
  const parsed = BigInt(normalized);
  if (parsed > MAX_NAT) {
    throw new Error(`${label} must fit in an unsigned 64-bit integer`);
  }
  return parsed;
}

export function add(sdk, left, right) {
  return sdk.prod_fixture_add(
    parseNat(left, "First number"),
    parseNat(right, "Second number"),
  );
}

export function less(sdk, left, right) {
  return sdk.prod_fixture_less(
    parseNat(left, "Left value"),
    parseNat(right, "Right value"),
  );
}

export function triggerOverflow(sdk) {
  return sdk.prod_fixture_riskyflag(MAX_NAT);
}

export function errorMessage(error) {
  return error instanceof Error ? error.message : String(error);
}
