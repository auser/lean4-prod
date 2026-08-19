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

export function wittBits(sdk, level) {
  return sdk.prod_uor_framework_wittbits(parseNat(level, "Witt level"));
}

export function wittBytes(sdk, level) {
  return sdk.prod_uor_framework_wittbytes(parseNat(level, "Witt level"));
}

export function addIsCommutative(sdk) {
  return sdk.prod_uor_framework_addiscommutative();
}

export function errorMessage(error) {
  return error instanceof Error ? error.message : String(error);
}
