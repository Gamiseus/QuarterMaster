// QuarterMaster site script. No dependencies; the only request it makes is for this site's own
// search.json. Every page works without it: the wiki sidebar stays open, the mod list is not filtered, code blocks have no
// Copy button, and the search page says what to use instead.
(function () {
  'use strict';

  // Wiki sidebar: open beside the text on wide screens, a closed menu on narrow ones.
  var side = document.querySelector('details.side');
  if (side) {
    var narrow = window.matchMedia('(max-width: 900px)');
    var fit = function () { side.open = !narrow.matches; };
    fit();
    narrow.addEventListener('change', fit);
    side.addEventListener('click', function (e) {
      if (narrow.matches && e.target.closest('a[href^="#"]')) side.open = false;
    });
  }

  // The mod list's filter: shows the cards whose name, summary, author or tags hold the text.
  var filter = document.getElementById('q');
  var cards = document.querySelectorAll('#mods .card');
  if (filter && cards.length) {
    var none = document.getElementById('none');
    filter.addEventListener('input', function () {
      var text = filter.value.trim().toLowerCase();
      var shown = 0;
      cards.forEach(function (card) {
        var ok = !text || card.getAttribute('data-search').indexOf(text) !== -1;
        card.hidden = !ok;
        if (ok) shown++;
      });
      none.hidden = shown !== 0;
    });
  }

  // A Copy button on each code block.
  if (navigator.clipboard) {
    document.querySelectorAll('pre.code').forEach(function (pre) {
      var wrap = document.createElement('div');
      wrap.className = 'codeblock';
      pre.parentNode.insertBefore(wrap, pre);
      wrap.appendChild(pre);
      var button = document.createElement('button');
      button.type = 'button';
      button.className = 'copy';
      button.textContent = 'Copy';
      button.addEventListener('click', function () {
        navigator.clipboard.writeText(pre.textContent).then(function () {
          button.textContent = 'Copied';
          setTimeout(function () { button.textContent = 'Copy'; }, 1500);
        });
      });
      wrap.appendChild(button);
    });
  }

  // Mark the section being read in the wiki sidebar.
  var sectionLinks = document.querySelectorAll('.side .secs a');
  if (sectionLinks.length && 'IntersectionObserver' in window) {
    var linkFor = {};
    sectionLinks.forEach(function (a) { linkFor[decodeURIComponent(a.hash.slice(1))] = a; });
    var marked = null;
    var observer = new IntersectionObserver(function (entries) {
      entries.forEach(function (entry) {
        if (!entry.isIntersecting || !linkFor[entry.target.id]) return;
        if (marked) marked.classList.remove('here');
        marked = linkFor[entry.target.id];
        marked.classList.add('here');
      });
    }, { rootMargin: '0px 0px -70% 0px' });
    document.querySelectorAll('.doc h2[id], .doc h3[id]').forEach(function (h) { observer.observe(h); });
  }

  // The search page: every word typed must appear in a section's title or text.
  var results = document.getElementById('results');
  if (results) {
    var box = document.getElementById('sq');
    var status = document.getElementById('sstatus');
    var root = results.getAttribute('data-root') || '';
    var records = [];
    var LIMIT = 40;

    // Appends `text` to `parent` with each match of a term in <mark>.
    var highlight = function (parent, text, terms) {
      var lower = text.toLowerCase();
      var i = 0;
      while (i < text.length) {
        var at = -1, len = 0;
        terms.forEach(function (t) {
          var p = lower.indexOf(t, i);
          if (p !== -1 && (at === -1 || p < at)) { at = p; len = t.length; }
        });
        if (at === -1) break;
        if (at > i) parent.appendChild(document.createTextNode(text.slice(i, at)));
        var mark = document.createElement('mark');
        mark.textContent = text.slice(at, at + len);
        parent.appendChild(mark);
        i = at + len;
      }
      if (i < text.length) parent.appendChild(document.createTextNode(text.slice(i)));
    };

    var search = function () {
      var query = box.value.trim();
      var terms = query.toLowerCase().split(/\s+/).filter(Boolean);
      results.textContent = '';
      history.replaceState(null, '', query ? '?q=' + encodeURIComponent(query) : location.pathname);
      if (!terms.length) {
        status.textContent = 'Type a word or two. Every word must match.';
        return;
      }
      var hits = [];
      records.forEach(function (r) {
        var score = 0;
        for (var k = 0; k < terms.length; k++) {
          var inTitle = r.title.indexOf(terms[k]) !== -1;
          var inText = r.text.indexOf(terms[k]) !== -1;
          if (!inTitle && !inText) return;
          score += (inTitle ? 10 : 0) + (inText ? 1 : 0);
        }
        if (r.title === terms.join(' ')) score += 20;
        hits.push({ record: r, score: score });
      });
      hits.sort(function (a, b) { return b.score - a.score || a.record.order - b.record.order; });
      status.textContent = hits.length === 0 ? 'Nothing matches.'
        : hits.length === 1 ? '1 result.'
        : hits.length > LIMIT ? hits.length + ' results. Showing the first ' + LIMIT + '.'
        : hits.length + ' results.';
      hits.slice(0, LIMIT).forEach(function (hit) {
        var d = hit.record.source;
        var li = document.createElement('li');
        var a = document.createElement('a');
        a.href = root + d.u;
        highlight(a, d.t, terms);
        li.appendChild(a);
        if (d.p && d.p !== d.t) {
          var where = document.createElement('span');
          where.className = 'muted small';
          where.textContent = ' in ' + d.p;
          li.appendChild(where);
        }
        if (d.x) {
          var first = -1;
          terms.forEach(function (t) {
            var p = hit.record.text.indexOf(t);
            if (p !== -1 && (first === -1 || p < first)) first = p;
          });
          var from = Math.max(0, (first === -1 ? 0 : first) - 60);
          var snippet = (from > 0 ? '…' : '') + d.x.slice(from, from + 200) + (from + 200 < d.x.length ? '…' : '');
          var p = document.createElement('p');
          highlight(p, snippet, terms);
          li.appendChild(p);
        }
        results.appendChild(li);
      });
    };

    box.value = new URLSearchParams(location.search).get('q') || '';
    status.textContent = 'Loading…';
    fetch(root + 'search.json')
      .then(function (response) {
        if (!response.ok) throw new Error(response.status);
        return response.json();
      })
      .then(function (json) {
        records = json.map(function (d, n) {
          return { source: d, order: n, title: d.t.toLowerCase(), text: d.x.toLowerCase() };
        });
        search();
        box.addEventListener('input', search);
      })
      .catch(function () { status.textContent = 'The search index could not be loaded.'; });
  }
})();
