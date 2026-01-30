let bar_meters = document.getElementById("bar_meters");
let bar_meter_tune = document.createElement("td");
let bar_meter_ind = document.createElement("td");
let bar_meter_load = document.createElement("td");
let temperature = document.createElement("td");
let temperature_data = document.createElement("h2");
temperature_data.setAttribute("style", "color: green")
temperature.appendChild(temperature_data);
let clock = document.createElement("td");
let clock_data = document.createElement("h3");
clock_data.setAttribute("style", "color: magenta");
clock.appendChild(clock_data);
let call_sign = document.createElement("td");
let call_sign_data = document.createElement("h1");
call_sign_data.setAttribute("style", "color: cyan");
call_sign.appendChild(call_sign_data);

// Move clock/callsign into the header placeholders (no behavior change)
const headerAddr = document.getElementById("header_addr");
if (headerAddr) { headerAddr.textContent = window.location.host; }

const headerClock = document.getElementById("header_clock");
if (headerClock) {
    headerClock.replaceChildren(clock_data);
    clock.style.display = "none";  // keep table layout stable
}

const headerCall = document.getElementById("header_callsign");
if (headerCall) {
    headerCall.replaceChildren(call_sign_data);
    call_sign.style.display = "none";  // keep table layout stable
}

bar_meters.appendChild(bar_meter_tune);
bar_meters.appendChild(bar_meter_ind);
bar_meters.appendChild(bar_meter_load);
bar_meters.appendChild(temperature);
bar_meters.appendChild(clock);
bar_meters.appendChild(call_sign);
let meter_tune = document.getElementById("meter_tune");
let meterReadingElement_tune = document.getElementById("tune");
let meter_ind = document.getElementById("meter_ind");
let meterReadingElement_ind = document.getElementById("ind");
let meter_load = document.getElementById("meter_load");
let meterReadingElement_load = document.getElementById("load");
let learn_update = new EventSource("/sse");
let last_meter_value_tune = 0;
let last_meter_value_ind = 0;
let last_meter_value_load = 0;
let timeStamp = Date.now();
let sleep = false;
let meter_values = {};
let meter_color = "";
let old_data = "";
let storeMode = false;
// tune, ind, load button configuration.
const storeBtn = document.getElementById("store_btn");
storeBtn.addEventListener("click", (event) => {
    storeMode = storeMode == false ? true : false;
    if (storeMode == true) {
        storeBtn.classList.add("hover_not_disabled");
    } else {
        removeStore();
    }
    console.log(bandSelectors.childNodes);
});

const tuneBtn = document.getElementById("tune_button");
const indBtn = document.getElementById("ind_button");
const loadBtn = document.getElementById("load_button");
const bandSelectors = document.getElementById("band_selectors");
    bandSelectors.replaceChildren();
const bands = ["M10", "M11", "M20", "M40", "M80"];
bands.forEach((band, i) => {
    console.log(band);
    console.log(i);
    removeStore();
    let btnBox = document.createElement("td");
    let btn = document.createElement("button");
    btn.classList.add("button");
    btn.setAttribute("id", band);
    btn.innerText = band.slice(1, 3) + band.slice(0, 1);
    btn.addEventListener("click", (event) => {
        if (storeMode == true) {
            removeStore();
            fetch(`/store/${band}`, {
                method: "POST",
            });
        } else {
            fetch(`/recall/${band}`, {
                method: "POST",
            });
        }
    });
    btnBox.appendChild(btn);
    bandSelectors.appendChild(btnBox);
});
const myButtons = document.querySelectorAll(".button");
const configBtn = document.getElementById("config_btn");
const saveBtn = document.getElementById("save_btn");
const statusBar = document.getElementById("status_bar");
const statusBarContents = document.createElement("h3");
// Power buttons at the bottom.
pwrBtsArr = ["Blwr", "Fil", "HV", "Oper"];
pwrBtnTable = document.getElementById("power_btns");
pwrBtnTableRow = document.createElement("tr");
pwrBtnTable.appendChild(pwrBtnTableRow);
pwrBtsArr.forEach((btn) => {
    const tableData = document.createElement("td");
    const newForm = document.createElement("form");
    newForm.setAttribute("class", "form");
    newForm.setAttribute("id", `sw_${btn}`);
    newForm.setAttribute("action", "#");
    const newLabel = document.createElement("label");
    newLabel.setAttribute("class", "switch");
    const newInput = document.createElement("input");
    newInput.setAttribute("name", btn);
    newInput.setAttribute("type", "checkbox");
    newInput.addEventListener("change", pwrBtnAction);
    const newSpan = document.createElement("span");
    newSpan.setAttribute("class", "slider round");
    newLabel.appendChild(newInput);
    newLabel.appendChild(newSpan);
    newForm.appendChild(newLabel);
    tableData.appendChild(newForm);
    pwrBtnTableRow.appendChild(tableData);
});
pwrBtnTable.appendChild(pwrBtnTableRow);
let oldPwrButtonData = "";
function pwrBtnAction(event) {
    let formData = new FormData();
    formData.append("ID", event.target.name);
    formData.append("value", event.target.checked ? "ON" : "OFF");
    if (event.target.name == "Fil" || event.target.name == "HV") {
        formData.append("delay", "OFF");
    }

    if (meter_values && meter_values.pwr_btns && meter_values.pwr_btns[event.target.name]) {
        console.log(meter_values.pwr_btns[event.target.name][0]);
    }
    if (event.target.name == "Fil" || event.target.name == "HV") {
        if (event.target.checked) {
            fetch("/pwr_btn", {
                method: "POST",
                body: formData,
            });
            setTimeout(() => {
                formData.set("delay", "ON");
                fetch("/pwr_btn", {
                    method: "POST",
                    body: formData,
                });
            }, 3000);
        } else {
            fetch("/pwr_btn", {
                method: "POST",
                body: formData,
            });
        }
    } else {
        fetch("/pwr_btn", {
            method: "POST",
            body: formData,
        });
    }
}

let configWindow;
statusBar.appendChild(statusBarContents);
configBtn.addEventListener("click", (event) => {
    console.log(event);
    configWindow = window.open(
        "/config",
        "Config-Page",
        "width=600, height=600",
    );
});
saveBtn.addEventListener("click", (event) => {
    formData = new FormData();
    console.log(event.target.name);
    formData.append(event.target.name, event.target.value);
    fetch("/stop", {
        method: "POST",
        body: formData,
    });
});
const formVals = ["tune", "ind", "load"];
let lastSelectorPosition = "";
let lastBandSelected = "";
myButtons.forEach((button, i) => {
    button.classList.add("active");
    if (i < 3) {
        const formData = new FormData();
        formData.append(formVals[i], "submit");
        console.log(formData);
        button.addEventListener("click", (event) => {
            fetch(`/selector/${formVals[i]}`, {
                method: "POST",
                body: formData,
            });
        });
        button.addEventListener("mousewheel", (event) => {
            const formData = new FormData();
            formData.append(formVals[i], event.wheelDelta);
            console.log(formData);
            fetch("/mousewheel", {
                method: "POST",
                body: formData,
            });
        });
    }
    button.addEventListener("mouseover", (event) => {
        event.target.classList.add("mouseover");
    });
    button.addEventListener("mouseout", (event) => {
        event.target.classList.remove("mouseover");
    });
});
let gauges = document.querySelectorAll(".gauge");
console.log(Date.now());
learn_update.onmessage = (e) => {
    if (e.data == "close") {
        learn_update.close();
    } else {
        meter_values = JSON.parse(e.data);
        if (configWindow != null) {
            configWindow.postMessage(
                meter_values.status,
                window.location.origin,
            );
        }
        temperature_data.innerText = "Temp:" +  Math.round(meter_values.temperature) + " C";
        clock_data.innerText = meter_values.time;
        call_sign_data.innerText = meter_values.call_sign;
        bar_meter_tune.innerHTML =
            "<p>Current value = " +
            meter_values.tune +
            ": <br><meter value=" +
            meter_values.tune +
            ' min="0" max= ' +
            meter_values.max.tune +
            ' low="0" high="800" optimum ="500" ></meter></p>';
        bar_meter_ind.innerHTML =
            "<p>Current value = " +
            meter_values.ind +
            ": <br><meter value=" +
            meter_values.ind +
            ' min="0" max= ' +
            meter_values.max.ind +
            ' low="0" high="800" optimum ="500" ></meter></p>';
        bar_meter_load.innerHTML =
            "<p>Current value = " +
            meter_values.load +
            ": <br><meter value=" +
            meter_values.load +
            ' min="0" max= ' +
            meter_values.max.load +
            ' low="0" high="800" optimum ="500" ></meter></p>';
        statusBarContents.innerText = `Status Bar: ${meter_values.status}`;
        console.log(meter_values.pwr_btns);
        if (meter_values.pwr_btns.Fil[1] == "ON") {
            pwrBtnTableRow.childNodes[1].childNodes[0].childNodes[0].childNodes[1].setAttribute(
                "style",
                "background-color: magenta;",
            );
        } else {
            pwrBtnTableRow.childNodes[1].childNodes[0].childNodes[0].childNodes[1].removeAttribute(
                "style",
            );
        }
        if (meter_values.pwr_btns.HV[1] == "ON") {
            pwrBtnTableRow.childNodes[2].childNodes[0].childNodes[0].childNodes[1].setAttribute(
                "style",
                "background-color: magenta;",
            );
        } else {
            pwrBtnTableRow.childNodes[2].childNodes[0].childNodes[0].childNodes[1].removeAttribute(
                "style",
            );
        }
        console.log("BAND!!!!");
        console.log(meter_values.band);
        console.log(typeof meter_values.band);
        console.log(meter_values.sw_pos);
        if (
            lastSelectorPosition !== meter_values.sw_pos ||
            lastBandSelected !== meter_values.band
        ) {
            sleep = false;
            removeStore();
            timeStamp = Date.now();
            myButtons.forEach((button) => {
                button.classList.remove("hover_not_disabled");
            });

            switch (meter_values.sw_pos) {
                case "Tune":
                    console.log("tune selected");
                    console.log(`ratio is: ${meter_values.ratio.tune}`);
                    tuneBtn.classList.add("hover_not_disabled");
                    break;
                case "Ind":
                    console.log("ind selected");
                    indBtn.classList.add("hover_not_disabled");
                    break;
                case "Load":
                    console.log("load selected");
                    loadBtn.classList.add("hover_not_disabled");
                    break;
            }

            if (meter_values.band != "") {
                document
                    .getElementById(meter_values.band)
                    .classList.add("hover_not_disabled");
            }
            lastSelectorPosition = meter_values.sw_pos;
            lastBandSelected = meter_values.band;
        }

        if (old_data == JSON.stringify(meter_values)) {
            if (Date.now() - timeStamp > 120000 && sleep === false) {
                sleep = true;
                console.log("time expired");
                let formData = new FormData();
                formData.append("action", "stop");
                fetch("/stop", {
                    method: "POST",
                    body: formData,
                });
            }
        } else {
            timeStamp = Date.now();
            old_data = JSON.stringify(meter_values);
        }
    }
};
window.addEventListener("message", (e) => {
    console.log(e);
});
function removeStore() {
    storeBtn.classList.remove("hover_not_disabled");
    storeMode = false;
}
function float2Int(val) {
    return val | 0;
}

function setColor(val) {
    if (val <= 100) {
        meter_color = "#0f0";
    } else if (val > 200 && val < 349) {
        meter_color = "#ff0";
    } else if (val >= 349) {
        meter_color = "#ff0000";
    }
    return meter_color;
}
function displayReading(val, ratio) {
    val = Math.round(val / ratio);
    let count = float2Int(val / 400);
    let meter_value = val - 400 * count;
    let color = setColor(meter_value);
    return [count, meter_value, color];
}
//startMeterAnimation();
// Function to animate the meter
setTimeout(startMeterAnimation, 1000);
function startMeterAnimation() {
    setInterval(() => {

          if (!meter_values || !meter_values.ratio) { return; }
meterReadingElement_tune.innerText = displayReading(
            meter_values.tune,
            meter_values.ratio.tune,
        )[0];
        meter_tune.style.background = `conic-gradient(${"#0f0"} ${displayReading(meter_values.tune, meter_values.ratio.tune)[1] * 0.9}deg, #fff 0deg)`;
        meterReadingElement_ind.innerText = displayReading(
            meter_values.ind,
            meter_values.ratio.ind,
        )[0];
        meter_ind.style.background = `conic-gradient(${"#0f0"} ${displayReading(meter_values.ind, meter_values.ratio.ind)[1] * 0.9}deg, #fff 0deg)`;
        meterReadingElement_load.innerText = displayReading(
            meter_values.load,
            meter_values.ratio.load,
        )[0];
        meter_load.style.background = `conic-gradient(${"#0f0"} ${displayReading(meter_values.load, meter_values.ratio.load)[1] * 0.9}deg, #fff 0deg)`;
        gauges.forEach((gauge, i) => {
            switch (i) {
                case 0:
                    gauge.style.setProperty(
                        "--value",
                        meter_values.plate_v / 10000,
                    );
                    gauge.innerHTML = Math.round(meter_values.plate_v) + "V";
                    break;
                case 1:
                    gauge.style.setProperty(
                        "--value",
                        meter_values.plate_a / 3,
                    );
                    gauge.innerHTML = Math.round(meter_values.plate_a) + "A";
                    break;
                case 2:
                    gauge.style.setProperty(
                        "--value",
                        meter_values.screen_a / 200,
                    );
                    gauge.innerHTML = Math.round(meter_values.screen_a) + "mA";
                    break;
                case 3:
                    gauge.style.setProperty(
                        "--value",
                        meter_values.grid_a / 50,
                    );
                    gauge.innerHTML = Math.round(meter_values.grid_a) + "mA";
                    break;
            }
        });
    }, 10);
}