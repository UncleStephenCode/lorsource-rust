(function () {
  'use strict';

  var SVG_NS = 'http://www.w3.org/2000/svg';
  var MONTHS = [
    'январь', 'февраль', 'март', 'апрель', 'май', 'июнь',
    'июль', 'август', 'сентябрь', 'октябрь', 'ноябрь', 'декабрь'
  ];

  function svgElement(name, attributes) {
    var element = document.createElementNS(SVG_NS, name);
    Object.keys(attributes || {}).forEach(function (key) {
      element.setAttribute(key, attributes[key]);
    });
    return element;
  }

  function intensityClass(count) {
    if (!count) return '';
    if (count < 8) return 'q1';
    if (count < 32) return 'q2';
    if (count < 64) return 'q3';
    if (count < 128) return 'q4';
    return 'q5';
  }

  function russianDate(date) {
    return new Intl.DateTimeFormat('ru-RU', {
      day: 'numeric', month: 'long', year: 'numeric'
    }).format(date);
  }

  function renderHeatmap(container, stats) {
    var range = window.matchMedia('(min-width: 768px)').matches ? 12 : 6;
    var cell = window.matchMedia('(min-width: 1024px)').matches ? 10 : 8;
    var gap = 2;
    var domainWidth = (cell + gap) * 6 + 8;
    var graphHeight = 18 + (cell + gap) * 7;
    var start = new Date();
    start = new Date(start.getFullYear(), start.getMonth() - range + 1, 1);

    var wrapper = document.createElement('div');
    wrapper.className = 'cal-heatmap-container';
    wrapper.style.overflowX = 'auto';
    var svg = svgElement('svg', {
      'class': 'graph',
      'width': String(domainWidth * range),
      'height': String(graphHeight),
      'role': 'img',
      'aria-label': 'Активность пользователя за последний год'
    });

    for (var monthOffset = 0; monthOffset < range; monthOffset += 1) {
      var month = new Date(start.getFullYear(), start.getMonth() + monthOffset, 1);
      var monthGroup = svgElement('g', {
        'class': 'graph-domain',
        'transform': 'translate(' + (monthOffset * domainWidth) + ',0)'
      });
      var label = svgElement('text', {'class': 'graph-label', 'x': '2', 'y': '10'});
      label.textContent = MONTHS[month.getMonth()];
      monthGroup.appendChild(label);

      var daysGroup = svgElement('g', {
        'class': 'graph-subdomain-group',
        'transform': 'translate(2,18)'
      });
      var daysInMonth = new Date(month.getFullYear(), month.getMonth() + 1, 0).getDate();
      var firstWeekday = month.getDay();

      for (var day = 1; day <= daysInMonth; day += 1) {
        var date = new Date(month.getFullYear(), month.getMonth(), day);
        var week = Math.floor((firstWeekday + day - 1) / 7);
        var weekday = date.getDay();
        var epochSeconds = Math.floor(date.getTime() / 1000);
        var count = Number(stats[String(epochSeconds)] || 0);
        var classes = ['graph-rect'];
        var intensity = intensityClass(count);
        if (intensity) classes.push(intensity, 'hover_cursor');

        var rect = svgElement('rect', {
          'class': classes.join(' '),
          'x': String(week * (cell + gap)),
          'y': String(weekday * (cell + gap)),
          'width': String(cell),
          'height': String(cell),
          'data-date': String(date.getTime()),
          'data-count': String(count)
        });
        var title = svgElement('title');
        title.textContent = russianDate(date) + (count ? ' — сообщений: ' + count : '');
        rect.appendChild(title);
        if (count > 0) {
          rect.addEventListener('click', function (event) {
            window.location.href = '/search.jsp?dt=' + event.currentTarget.dataset.date +
              '&user=' + encodeURIComponent(container.dataset.searchUser);
          });
        }
        daysGroup.appendChild(rect);
      }

      monthGroup.appendChild(daysGroup);
      svg.appendChild(monthGroup);
    }

    wrapper.appendChild(svg);
    container.replaceChildren(wrapper);
  }

  async function initialize() {
    var container = document.getElementById('cal-heatmap');
    if (!container || !container.dataset.statsUrl) return;

    var timezone = Intl.DateTimeFormat().resolvedOptions().timeZone;
    if (timezone && timezone !== 'Factory' && timezone !== 'Etc/Unknown') {
      document.cookie = 'tz=' + encodeURIComponent(timezone) + '; Path=/; Max-Age=31536000; SameSite=Lax';
    }

    try {
      var response = await fetch(container.dataset.statsUrl, {
        headers: {'Accept': 'application/json'},
        credentials: 'same-origin'
      });
      if (!response.ok) throw new Error(response.statusText);
      renderHeatmap(container, await response.json());
    } catch (error) {
      container.textContent = 'Не удалось загрузить календарь активности';
      container.className = 'error';
    }
  }

  document.addEventListener('DOMContentLoaded', initialize);
}());
