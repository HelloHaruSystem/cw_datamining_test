// Theme toggle: system -> light -> dark -> system. Pages work without JS;
// this only adds the manual override (the button stays hidden otherwise).
(function () {
  var order = ["system", "light", "dark"];
  var names = { system: "System theme", light: "Light theme", dark: "Dark theme" };
  var root = document.documentElement;

  function current() {
    return root.dataset.theme || "system";
  }

  function apply(theme) {
    if (theme === "system") delete root.dataset.theme;
    else root.dataset.theme = theme;
    try {
      if (theme === "system") localStorage.removeItem("theme");
      else localStorage.setItem("theme", theme);
    } catch (e) {}
    document.querySelectorAll("[data-theme-toggle]").forEach(function (btn) {
      btn.querySelector(".theme-label").textContent = names[theme];
      btn.dataset.mode = theme;
      btn.setAttribute("aria-label", names[theme] + " (click to change)");
    });
  }

  document.querySelectorAll("[data-theme-toggle]").forEach(function (btn) {
    btn.hidden = false;
    btn.addEventListener("click", function () {
      apply(order[(order.indexOf(current()) + 1) % order.length]);
    });
  });
  apply(current());
})();

// Selects marked data-autosubmit submit their form on change (the form
// still has a plain submit button for no-JS visitors).
document.querySelectorAll("select[data-autosubmit]").forEach(function (sel) {
  sel.addEventListener("change", function () { sel.form.submit(); });
});
