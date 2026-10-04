// Skill builder. The page is rendered by the server; this adds the
// interaction. Build state lives in the URL hash (#b=id.lv,id.lv) so links
// can be shared; job, level and version live in the query string.
(function () {
  var dataEl = document.getElementById("builder-data");
  if (!dataEl) return;
  var data = JSON.parse(dataEl.textContent);

  var skills = {};
  data.skills.forEach(function (s) { skills[s.id] = s; });
  var levels = {};
  var level = data.level;

  var $ = function (sel, root) { return (root || document).querySelector(sel); };
  var $$ = function (sel, root) { return Array.prototype.slice.call((root || document).querySelectorAll(sel)); };

  // ---- state ---------------------------------------------------------------

  function readHash() {
    var m = /(?:^|[#&])b=([^&]*)/.exec(location.hash);
    if (!m) return;
    decodeURIComponent(m[1]).split(",").forEach(function (pair) {
      var p = pair.split(/[.:]/);
      var s = skills[p[0]];
      var lv = parseInt(p[1], 10);
      if (s && lv > 0) levels[s.id] = Math.min(lv, s.max);
    });
  }

  function lv(id) { return levels[id] || 0; }

  function poolIndex(job) { return data.path.indexOf(job); }
  function poolTotal(job) { return data.pools[level - 1][poolIndex(job)] || 0; }
  function poolSpent(job) {
    return data.skills.reduce(function (sum, s) { return s.job === job ? sum + lv(s.id) : sum; }, 0);
  }

  function reqsMet(s) {
    return Object.keys(s.req).every(function (r) { return lv(r) >= s.req[r]; });
  }

  // Lowering `id` to `to` must not break another skill's requirement.
  function blocksLowering(id, to) {
    return data.skills.some(function (o) {
      return lv(o.id) > 0 && o.req[id] !== undefined && o.req[id] > to;
    });
  }

  function canAdd(s) {
    return lv(s.id) < s.max && reqsMet(s) && poolSpent(s.job) < poolTotal(s.job);
  }

  function canRemove(s) {
    return lv(s.id) > 0 && !blocksLowering(s.id, lv(s.id) - 1);
  }

  function addReason(s) {
    if (lv(s.id) >= s.max) return "Maxed";
    if (!reqsMet(s)) {
      return "Needs " + Object.keys(s.req).filter(function (r) { return lv(r) < s.req[r]; })
        .map(function (r) { return (skills[r] ? skills[r].name : r) + " " + s.req[r]; }).join(", ");
    }
    if (poolSpent(s.job) >= poolTotal(s.job)) return "No " + data.names[s.job] + " SP left";
    return "";
  }

  // ---- url -----------------------------------------------------------------

  function buildString() {
    return data.skills.filter(function (s) { return lv(s.id) > 0; })
      .map(function (s) { return s.id + "." + lv(s.id); }).join(",");
  }

  function url() {
    var params = new URLSearchParams();
    if (data.pinned) params.set("v", data.version);
    params.set("job", data.job);
    params.set("lv", level);
    var b = buildString();
    return location.pathname + "?" + params.toString() + (b ? "#b=" + b : "");
  }

  function syncUrl() {
    history.replaceState(null, "", url());
  }

  // ---- render --------------------------------------------------------------

  // Meters animate only for changes made after the page loaded.
  var initialized = false;

  function render() {
    if (initialized) $("[data-builder-pools]").classList.add("ready");
    var problems = [];
    data.path.forEach(function (job) {
      var spent = poolSpent(job), total = poolTotal(job);
      var el = $('[data-pool="' + job + '"]');
      if (!el) return;
      $("[data-pool-spent]", el).textContent = spent;
      $("[data-pool-total]", el).textContent = total;
      var fill = $("[data-pool-meter]", el);
      fill.style.width = (total ? Math.min(100, (spent / total) * 100) : 0) + "%";
      el.classList.toggle("over", spent > total);
      el.classList.toggle("full", spent === total && total > 0);
      if (spent > total) problems.push(data.names[job] + " uses " + spent + " SP but only " + total + " are available at level " + level + ".");
    });

    $$("[data-skill]").forEach(function (card) {
      var s = skills[card.getAttribute("data-skill")];
      if (!s) return;
      var cur = lv(s.id);
      $("[data-skill-level]", card).textContent = cur;
      var add = canAdd(s), reason = add ? "" : addReason(s);
      $$("[data-step]", card).forEach(function (btn) {
        var step = btn.getAttribute("data-step");
        btn.disabled = step === "-1" ? !canRemove(s) : !add;
        btn.title = step === "-1" ? (canRemove(s) || cur === 0 ? "" : "Another skill needs this level") : reason;
      });
      card.classList.toggle("active", cur > 0);
      card.classList.toggle("maxed", cur === s.max);
      card.classList.toggle("locked", cur === 0 && !reqsMet(s));
      var eff = $("[data-skill-effect]", card);
      var html = "";
      if (cur > 0 && s.texts[cur]) html += '<span class="muted">Lv ' + cur + ":</span> " + s.texts[cur];
      if (cur < s.max && s.texts[cur + 1]) {
        html += (html ? "<br>" : "") + '<span class="muted">' + (cur ? "Next" : "Lv 1") + ":</span> " + s.texts[cur + 1];
      }
      eff.innerHTML = html;
    });

    var box = $("[data-builder-problems]");
    box.hidden = problems.length === 0;
    box.textContent = problems.join(" ");
    syncUrl();
  }

  function status(msg) {
    var el = $("[data-builder-status]");
    el.textContent = msg;
    if (msg) setTimeout(function () { if (el.textContent === msg) el.textContent = ""; }, 2500);
  }

  // ---- events --------------------------------------------------------------

  document.addEventListener("click", function (e) {
    var btn = e.target.closest("[data-step]");
    if (btn && !btn.disabled) {
      var s = skills[btn.closest("[data-skill]").getAttribute("data-skill")];
      var step = btn.getAttribute("data-step");
      if (step === "1") levels[s.id] = lv(s.id) + 1;
      else if (step === "-1") levels[s.id] = lv(s.id) - 1;
      else while (canAdd(s)) levels[s.id] = lv(s.id) + 1;
      render();
      return;
    }
    var reset = e.target.closest("[data-builder-reset]");
    if (reset) {
      var job = parseInt(reset.getAttribute("data-builder-reset"), 10);
      data.skills.forEach(function (s) { if (s.job === job) delete levels[s.id]; });
      render();
      status(data.names[job] + " reset");
    }
  });

  var levelInput = $("[data-builder-level]");
  var levelOut = $("[data-builder-level-out]");
  levelInput.addEventListener("input", function () {
    level = parseInt(levelInput.value, 10);
    levelOut.textContent = level;
    render();
  });

  $("[data-builder-job]").addEventListener("change", function (e) {
    var params = new URLSearchParams();
    if (data.pinned) params.set("v", data.version);
    params.set("job", e.target.value);
    params.set("lv", level);
    var b = buildString();
    location.href = location.pathname + "?" + params.toString() + (b ? "#b=" + b : "");
  });

  $("[data-builder-reset-all]").addEventListener("click", function () {
    levels = {};
    render();
    status("Build reset");
  });

  $("[data-builder-share]").addEventListener("click", function () {
    var link = location.origin + url();
    if (navigator.clipboard) {
      navigator.clipboard.writeText(link).then(function () { status("Link copied"); },
        function () { status(link); });
    } else {
      status(link);
    }
  });

  $$("[data-builder-share], [data-builder-reset-all], [data-builder-reset]").forEach(function (b) { b.hidden = false; });
  readHash();
  render();
  initialized = true;
})();
