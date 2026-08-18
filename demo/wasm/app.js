import init, * as sdk from "../../output/fixture/wasm/fixture.js";
import { add, errorMessage, less, triggerOverflow } from "./demo-core.mjs";

const engineStatus = document.querySelector(".engine-status");
const statusText = document.querySelector("#engine-status");
const controls = document.querySelectorAll("button, input");

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

document.querySelector("#add-form").addEventListener("submit", (event) => {
  event.preventDefault();
  run(document.querySelector("#add-result"), () =>
    add(
      sdk,
      document.querySelector("#add-left").value,
      document.querySelector("#add-right").value,
    ),
  );
});

document.querySelector("#less-form").addEventListener("submit", (event) => {
  event.preventDefault();
  run(
    document.querySelector("#less-result"),
    () =>
      less(
        sdk,
        document.querySelector("#less-left").value,
        document.querySelector("#less-right").value,
      ),
    (value) => (value ? "True" : "False"),
  );
});

document.querySelector("#overflow-button").addEventListener("click", () => {
  run(document.querySelector("#overflow-result"), () => triggerOverflow(sdk));
});
