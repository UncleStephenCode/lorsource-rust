/* Tango-only dark/light/auto switcher, matching the original LOR contract. */
(function () {
  function initThemeSwitcher() {
    var indicator = document.getElementById('theme-indicator');
    if (!indicator || !document.documentElement.dataset.style.startsWith('tango')) return;
    var themes = ['dark', 'light', 'auto'];
    indicator.addEventListener('click', function () {
      var html = document.documentElement;
      var index = themes.indexOf(html.getAttribute('data-theme'));
      if (index === -1) return;
      var next = themes[(index + 1) % themes.length];
      html.setAttribute('data-theme', next);
      localStorage.setItem('lor-theme', next);
    });
  }
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', initThemeSwitcher);
  else initThemeSwitcher();
})();
