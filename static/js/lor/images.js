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
  // fix images on Pale Moon
  $('.medium-image-container').each(function () {
    if ($(this).width() == 0) {
      $(this).css('width', 'var(--image-width)')
    }
  });
  $('.slider-parent').each(function () {
    if ($(this).height() <= 48) {
      $(this).css('width', 'var(--image-width)')
    }
  });

  $script.ready('plugins', function () {
    if (window.matchMedia("(min-width: 70em)").matches) {
      $(".msg_body .swiffy-slider").addClass("slider-nav-outside-expand").addClass("slider-nav-visible");
    }

    $(".slider-indicators a").attr('href', 'javascript:;');

    window.swiffyslider.init();
  });
});