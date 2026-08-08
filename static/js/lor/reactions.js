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

function initReactionsUI() {
  $script.ready('plugins', function () {
    twemoji.parse(document.body);

    $(".reaction-anonymous").prop("disabled", false).each(function () {
      tippy(this, {
        content: "Для добавления реакции нужно залогиниться!",
        trigger: 'manual'
      });
    }).on("click", function (event) {
      event.preventDefault();
      event.stopPropagation();
      this._tippy.show();
    });
  });

  $('.reaction-show').on('click', function (event) {
    event.preventDefault();

    const reactions = $(this).parents('.msg_body').find('.reactions');

    if (reactions.is(":hidden") || reactions.find('.zero-reactions').is(":hidden")) {
      $('.zero-reactions').hide();
      $('.reactions .reaction-show').html("&raquo;");

      if (reactions.hasClass("zero-reactions")) {
        reactions.show();
      } else {
        reactions.find('.zero-reactions').show();
        $(this).html("&laquo;");
      }
    } else {
      $('.zero-reactions').hide();
      $('.reactions .reaction-show').html("&raquo;");
    }
  })

  $script.ready('plugins', function () {
    $('button.reaction').not(".reaction-anonymous").on('click', function (event) {
      event.preventDefault();

      const value = $(this).attr('value');
      const btn = $(this);
      const form = $(this).parents(".reactions-form")
      const reactions = $(this).parents('.msg_body').find('.reactions form');

      $(reactions).find(".error").remove();

      const options = {
        url: "/reactions/ajax",
        data: {"reaction": value},
        success: function (response) {
          reactions.parents(".zero-reactions").removeClass("zero-reactions")

          btn.find('.reaction-count').text(response.count);

          if (value.endsWith('-true')) {
            form.find("button.btn-primary").each(function () {
              $(this).attr('value', $(this).attr('value').replace(/-.*/, "-true"));
              $(this).find(".reaction-count").text($(this).find(".reaction-count").text() - 1);
              $(this).removeClass("btn-primary");
            });

            btn.attr('value', value.replace(/-.*/, "-false"));
            btn.addClass("btn-primary");
          } else {
            btn.attr('value', value.replace(/-.*/, "-true"));
            btn.removeClass("btn-primary");
          }
        },
        error: function (jqXHR, textStatus, errorThrown) {
          reactions.append(
              $("<div class=error>")
                  .text("Возможно, что превышен лимит реакций. Попробуйте снова через 10 минут. " + errorThrown)
          );
        }
      };

      const formData = $(this).parents('.reactions-form').serializeArray();
      formData.push({name: 'reaction', value: value});
      $.ajax($.extend(options, {
        method: 'POST',
        data: $.param(formData)
      }));
    });
  })
}

$(document).ready(function () {
  initReactionsUI();
});