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

$(document).ready(function () {
  function spoilerShow() {
    const $this = $(this);
    $(this).closest('.spoiled').removeClass('spoiled').addClass("unspoiled");
    $this.remove();
    return false;
  }

  function initCodeSpoilers() {
    $('div.code').each(function () {
      if (this.scrollHeight > this.clientHeight) {
        $(this)
            .append($('<div class="spoiler-open"><span class="btn btn-small btn-default spoiler-button">Развернуть</span></div> ').on('click', spoilerShow))
            .addClass('spoiled');
      }
    });
  }

  initCodeSpoilers();
});