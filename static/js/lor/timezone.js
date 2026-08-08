/*
 * Copyright 1998-2026 Linux.org.ru
 *    Licensed under the Apache License, Version 2.0 (the "License");
 *    you may not use this file except in compliance with the License.
 *    You may obtain a copy of the License at
 *
 *        http://www.apache.org/licenses/LICENSE-2.0
 *
 *    Unless required by applicable law or agreed to in writing, software
 *    distributed under the License is distributed on an "AS IS" BASIS,
 *    WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 *    See the License for the specific language governing permissions and
 *    limitations under the License.
 */

function fixTimezone(serverTz) {
  const tz = Intl.DateTimeFormat().resolvedOptions().timeZone;

  if (typeof tz !== 'undefined') {
    $script.ready('plugins', function () {
      if (Cookies.get('tz') !== tz) {
        Cookies.set('tz', tz, {expires: 365})
      }

      if (tz !== serverTz) {
        $(function () {
          $("time[data-format]").each(function () {
            const date = Date.parse($(this).attr("datetime"));

            const format = $(this).attr("data-format");

            const diff = Date.now() - date;
            const today = new Date().setHours(0, 0, 0, 0);
            const yesterdayTs = new Date(today);
            yesterdayTs.setDate(yesterdayTs.getDate() - 1);
            const min = Math.floor(diff / (1000 * 60))

            if (format === 'default') {
              $(this).text(moment(date).format("DD.MM.yy HH:mm:ss Z"));
            } else if (format === 'date') {
              $(this).text(moment(date).format("DD.MM.yy"));
            } else if (format === 'compact-interval') {
              if (diff < 1000 * 60 * 60) {
                $(this).text(Math.max(1, min) + "\xA0мин");
              } else if (diff < 1000 * 60 * 60 * 4 || date >= today) {
                $(this).text(moment(date).format("HH:mm"));
              } else if (date >= yesterdayTs) {
                $(this).text("вчера")
              } else {
                $(this).text(moment(date).format("DD.MM.yy"));
              }
            } else if (format === 'interval') {
              if (diff < 2 * 1000 * 60) {
                $(this).text("минуту назад");
              } else if (diff < 1000 * 60 * 60) {
                if (min % 10 < 5 && min % 10 > 1 && (min > 20 || min < 10)) {
                  $(this).text(min + "\xA0минуты назад");
                } else if (min % 10 === 1 && min > 20) {
                  $(this).text(min + "\xA0минута назад");
                } else {
                  $(this).text(min + "\xA0минут назад");
                }
              } else if (date >= today) {
                $(this).text("сегодня " + moment(date).format("HH:mm"));
              } else if (date >= yesterdayTs) {
                $(this).text("вчера " + moment(date).format("HH:mm"));
              } else {
                $(this).text(moment(date).format("DD.MM.yy HH:mm"));
              }
            }
          });
        })
      }
    });
  }
}