// The hero terminal: an encrypted .pathrc.toml.age, `cd` into its directory,
// and the cd hook's shell code scrambling out of the ciphertext; then `cd ..`
// undoing it. The hook lines are what `pathrc collate` prints for the shell
// to evaluate (it does so silently; they are shown here to make it visible).
// With reduced motion the finished transcript is shown, without animation.

(function () {
  "use strict";

  var term = document.getElementById("terminal");
  if (!term) return;

  var top = "~/work/aws-project";
  var prod = "~/work/aws-project/prod";
  var cipherChars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/=";

  // Each step: a typed command, printed lines, a scrambled line or a pause.
  var script = [
    { cmd: "cat prod/.pathrc.toml.age", at: top },
    { out: "-----BEGIN AGE ENCRYPTED FILE-----", cls: "cipher" },
    { out: "YWdlLWVuY3J5cHRpb24ub3JnL3YxCi0+IFgy", cls: "cipher" },
    { out: "NTUxOSBuV2dIcUZ3b1k2eVBrdHJ6TmFmT2h6", cls: "cipher" },
    { out: "-----END AGE ENCRYPTED FILE-----", cls: "cipher" },
    { pause: 700 },
    { cmd: "cd prod", at: top },
    { out: "  # cd hook: .pathrc.toml.age decrypted in memory", cls: "note" },
    { scramble: "  export AWS_ACCOUNT_ID='111111111111'", cls: "set" },
    { scramble: "  export AWS_REGION='eu-west-1'", cls: "set" },
    { pause: 500 },
    { cmd: "echo $AWS_ACCOUNT_ID", at: prod },
    { out: "111111111111", cls: "value" },
    { pause: 900 },
    { cmd: "cd ..", at: prod },
    { out: "  # leaving prod: what it set is undone", cls: "note" },
    { out: "  unset AWS_ACCOUNT_ID", cls: "unset" },
    { out: "  export AWS_REGION='eu-west-2'", cls: "set" },
    { pause: 400 },
    { cmd: 'echo "[$AWS_ACCOUNT_ID]"', at: top },
    { out: "[]", cls: "value" },
    { prompt: top },
  ];

  function el(tag, cls, text) {
    var node = document.createElement(tag);
    if (cls) node.className = cls;
    if (text !== undefined) node.textContent = text;
    return node;
  }

  function line() {
    var div = el("div");
    term.appendChild(div);
    term.scrollTop = term.scrollHeight;
    return div;
  }

  function prompt(div, at) {
    div.appendChild(el("span", "d", at));
    div.appendChild(el("span", "p", " $ "));
  }

  // The whole transcript at once.
  function renderStatic() {
    term.textContent = "";
    script.forEach(function (step) {
      if (step.cmd) {
        var div = line();
        prompt(div, step.at);
        div.appendChild(document.createTextNode(step.cmd));
      } else if (step.out || step.scramble) {
        line().appendChild(el("span", step.cls, step.out || step.scramble));
      } else if (step.prompt) {
        prompt(line(), step.prompt);
      }
    });
  }

  var reduce = window.matchMedia("(prefers-reduced-motion: reduce)");
  if (reduce.matches) {
    renderStatic();
    return;
  }

  var timers = [];
  function later(fn, ms) {
    timers.push(setTimeout(fn, ms));
  }

  function randomCipher(n) {
    var s = "";
    for (var i = 0; i < n; i++) {
      s += cipherChars.charAt(Math.floor(Math.random() * cipherChars.length));
    }
    return s;
  }

  function run(index) {
    if (index >= script.length) {
      later(function () {
        timers = [];
        term.textContent = "";
        run(0);
      }, 5000);
      return;
    }
    var step = script[index];
    var next = function () {
      run(index + 1);
    };

    if (step.pause) {
      later(next, step.pause);
    } else if (step.cmd) {
      var div = line();
      prompt(div, step.at);
      var typed = el("span");
      var cursor = el("span", "cursor");
      div.appendChild(typed);
      div.appendChild(cursor);
      var i = 0;
      (function type() {
        if (i < step.cmd.length) {
          typed.textContent += step.cmd.charAt(i++);
          later(type, 45 + Math.random() * 55);
        } else {
          later(function () {
            div.removeChild(cursor);
            next();
          }, 350);
        }
      })();
    } else if (step.out) {
      line().appendChild(el("span", step.cls, step.out));
      later(next, 90);
    } else if (step.scramble) {
      // Ciphertext that resolves, left to right, into the hook's code.
      var target = step.scramble;
      var span = el("span", "scramble");
      line().appendChild(span);
      var frame = 0;
      var frames = 26;
      (function tick() {
        var shown = Math.floor((frame / frames) * target.length);
        span.textContent = target.slice(0, shown) + randomCipher(target.length - shown);
        if (frame++ < frames) {
          later(tick, 38);
        } else {
          span.textContent = target;
          span.className = step.cls;
          later(next, 160);
        }
      })();
    } else if (step.prompt) {
      var last = line();
      prompt(last, step.prompt);
      last.appendChild(el("span", "cursor"));
      next();
    }
  }

  run(0);

  // If the user turns on reduced motion while the page is open, stop.
  if (reduce.addEventListener) {
    reduce.addEventListener("change", function (e) {
      if (e.matches) {
        timers.forEach(clearTimeout);
        renderStatic();
      }
    });
  }
})();

// Copy buttons on the install commands.
(function () {
  "use strict";
  var buttons = document.querySelectorAll("button.copy");
  Array.prototype.forEach.call(buttons, function (button) {
    button.addEventListener("click", function () {
      var text = button.getAttribute("data-copy");
      var done = function (label) {
        button.textContent = label;
        setTimeout(function () {
          button.textContent = "Copy";
        }, 1600);
      };
      if (navigator.clipboard && navigator.clipboard.writeText) {
        navigator.clipboard.writeText(text).then(
          function () {
            done("Copied");
          },
          function () {
            done("Select and copy");
          }
        );
      } else {
        done("Select and copy");
      }
    });
  });
})();
