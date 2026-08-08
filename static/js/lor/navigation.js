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

function initNextPrevKeys() {
  $script.ready('plugins', function () {
    function jump(link) {
      if (link && link.href) {
        document.location = link.href;
      }
    }

    if (typeof jQuery.hotkeys !== 'undefined') {
      $(document).on('keydown', {combi: 'Ctrl+left', disableInInput: true}, function () {
        jump(document.getElementById('PrevLink'))
      });
      $(document).on('keydown', {combi: 'Ctrl+right', disableInInput: true}, function () {
        jump(document.getElementById('NextLink'))
      });
    }
  });
}

function replace_state() {
  if (typeof (history.replaceState) !== 'function') return;

  if (document.location.hash.indexOf('#comment-') === 0 && !document.location.pathname.startsWith("/view-deleted")) {
    if (document.getElementById(document.location.hash.substring(1)) === null) return;

    if (document.querySelector('article.msg:target') === null) {
      setTimeout(replace_state, 50);
      return;
    }

    const hash = document.location.hash.split('-');
    if (parseInt(hash[1]) > 0) {
      const p = document.location.pathname.split('/');
      const pathname = [p[0], p[1], p[2], p[3]].join('/');
      history.replaceState(null, document.title, pathname + '?cid=' + hash[1]);
    }
  }
}

$(document).ready(function () {
  function initSamepageCommentNavigation() {
    $("article.msg a[data-samepage=true]").on("click", function (event) {
      event.preventDefault();
      location.hash = "comment-" + this.search.match(/cid=(\d+)/)[1];
    })
  }

  function initScollupButton() {
    const backButton = $('<button id="ft-back-button">');

    backButton.text("Вверх");

    backButton.on("click", function () {
      $("html, body").animate({scrollTop: 0});
    });

    $('#ft').prepend(backButton);
  }

  initSamepageCommentNavigation();
  initScollupButton();

  replace_state();
  $(window).on('hashchange', replace_state);
});