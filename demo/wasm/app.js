import init, * as sdk from "../../output/uor/wasm/uor.js";
import {
  addIsCommutative,
  errorMessage,
  wittBits,
  wittBytes,
} from "./demo-core.mjs";

const engineStatus = document.querySelector(".engine-status");
const statusText = document.querySelector("#engine-status");
const controls = document.querySelectorAll("button, input, select");

function setControlsDisabled(disabled) {
  for (const control of controls) control.disabled = disabled;
}

function showResult(output, value, failed = false) {
  output.value = value;
  output.textContent = value;
  output.classList.toggle("success", !failed);
  output.classList.toggle("failure", failed);
}

function run(output, operation, format = String) {
  try {
    showResult(output, format(operation()));
  } catch (error) {
    showResult(output, `Error: ${errorMessage(error)}`, true);
  }
}

setControlsDisabled(true);

try {
  await init();
  engineStatus.classList.add("ready");
  statusText.textContent = "Generated WebAssembly loaded";
  setControlsDisabled(false);
} catch (error) {
  engineStatus.classList.add("error");
  statusText.textContent = `WebAssembly failed to load: ${errorMessage(error)}`;
}

document.querySelector("#witt-form").addEventListener("submit", (event) => {
  event.preventDefault();
  const level = document.querySelector("#witt-level").value;
  run(
    document.querySelector("#witt-result"),
    () => ({ bits: wittBits(sdk, level), bytes: wittBytes(sdk, level) }),
    ({ bits, bytes }) => `W${bits} = ${bytes} bytes`,
  );
});

document.querySelector("#metadata-button").addEventListener("click", () => {
  run(
    document.querySelector("#metadata-result"),
    () => addIsCommutative(sdk),
    (value) => `PrimitiveOp.add.isCommutative = ${value ? "true" : "false"}`,
  );
});
