// Progressive enhancement only: without JS the docs remain fully navigable.
(function () {
  'use strict';

  // ── Component search ───────────────────────────────────────────────────────
  var input = document.getElementById('docs-search');
  var panel = document.getElementById('search-results');
  var list = document.getElementById('search-results-list');
  var status = document.getElementById('search-status');
  if (input && panel && list && status) {
    var index = null;
    var base = panel.getAttribute('data-base') || '';
    var load = function () {
      if (index) return Promise.resolve(index);
      return fetch(base + 'search-index.json', { credentials: 'same-origin' })
        .then(function (response) { return response.json(); })
        .then(function (data) { index = data; return data; });
    };
    var render = function (query) {
      var needle = query.trim().toLowerCase();
      if (!needle) {
        panel.hidden = true;
        list.innerHTML = '';
        status.textContent = '';
        return;
      }
      load().then(function (data) {
        var matches = [];
        data.pages.forEach(function (page) {
          if (page.title.toLowerCase().indexOf(needle) !== -1) {
            matches.push({ href: base + page.href, label: page.title + ' (component)' });
          }
          page.headings.forEach(function (heading) {
            if (heading.text.toLowerCase().indexOf(needle) !== -1) {
              matches.push({ href: base + page.href + '#' + heading.id, label: page.title + ' › ' + heading.text });
            }
          });
        });
        var top = matches.slice(0, 12);
        list.innerHTML = top.map(function (match) {
          return '<li><a href="' + match.href + '">' + match.label.replace(/[<>&]/g, '') + '</a></li>';
        }).join('');
        panel.hidden = top.length === 0;
        status.textContent = top.length === 0
          ? 'No documentation matched "' + query.trim() + '".'
          : matches.length + ' result' + (matches.length === 1 ? '' : 's') + ' for "' + query.trim() + '".';
      });
    };
    var timer = null;
    input.addEventListener('input', function () {
      window.clearTimeout(timer);
      timer = window.setTimeout(function () { render(input.value); }, 120);
    });
    input.addEventListener('keydown', function (event) {
      if (event.key === 'Escape') { input.value = ''; render(''); }
    });
  }

  // ── Version selector ───────────────────────────────────────────────────────
  var versionSelect = document.getElementById('version-select');
  if (versionSelect) {
    versionSelect.addEventListener('change', function () {
      if (versionSelect.value) window.location.href = versionSelect.value;
    });
  }

  // ── Copy button for code blocks ────────────────────────────────────────────
  Array.prototype.forEach.call(document.querySelectorAll('main pre'), function (block, index) {
    var code = block.querySelector('code');
    if (!code || !navigator.clipboard) return;
    var id = 'code-' + index;
    block.setAttribute('id', id);
    var button = document.createElement('button');
    button.type = 'button';
    button.textContent = 'Copy';
    button.className = 'copy-button';
    button.addEventListener('click', function () {
      navigator.clipboard.writeText(code.textContent || '').then(function () {
        button.textContent = 'Copied';
        window.setTimeout(function () { button.textContent = 'Copy'; }, 1500);
      });
    });
    block.parentNode.insertBefore(button, block);
  });
})();
