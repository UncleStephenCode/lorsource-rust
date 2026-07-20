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

/* The Java application initializes Swiffy through jQuery/$script.  The Rust
 * port keeps the same DOM and CSS but initializes it without those global
 * dependencies, which are not otherwise needed by the server-rendered UI. */
document.addEventListener('DOMContentLoaded', function () {
  document.querySelectorAll('.medium-image-container').forEach(function (element) {
    if (element.getBoundingClientRect().width === 0) {
      element.style.width = 'var(--image-width)';
    }
  });

  document.querySelectorAll('.slider-parent').forEach(function (element) {
    if (element.getBoundingClientRect().height <= 48) {
      element.style.width = 'var(--image-width)';
    }
  });

  if (window.matchMedia('(min-width: 70em)').matches) {
    document.querySelectorAll('.msg_body .swiffy-slider').forEach(function (slider) {
      slider.classList.add('slider-nav-outside-expand', 'slider-nav-visible');
    });
  }

  document.querySelectorAll('.swiffy-slider').forEach(function (slider) {
    var container = slider.querySelector('.slider-container');
    if (!container) return;

    var slides = Array.from(container.children);
    var indicators = Array.from(slider.querySelectorAll('.slider-indicators a'));
    var previous = slider.querySelector('.slider-nav:not(.slider-nav-next)');
    var next = slider.querySelector('.slider-nav-next');

    function currentIndex() {
      if (!container.clientWidth) return 0;
      return Math.max(0, Math.min(slides.length - 1, Math.round(container.scrollLeft / container.clientWidth)));
    }

    function show(index) {
      if (!slides.length) return;
      index = (index + slides.length) % slides.length;
      container.scrollTo({left: slides[index].offsetLeft, behavior: 'smooth'});
      indicators.forEach(function (indicator, item) {
        indicator.classList.toggle('active', item === index);
      });
    }

    if (previous) previous.addEventListener('click', function () { show(currentIndex() - 1); });
    if (next) next.addEventListener('click', function () { show(currentIndex() + 1); });
    indicators.forEach(function (indicator, index) {
      indicator.addEventListener('click', function (event) {
        event.preventDefault();
        show(index);
      });
    });
    container.addEventListener('scroll', function () {
      var index = currentIndex();
      indicators.forEach(function (indicator, item) {
        indicator.classList.toggle('active', item === index);
      });
    }, {passive: true});
  });
});
