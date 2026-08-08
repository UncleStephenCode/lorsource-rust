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

function initStarPopovers() {
  $script.ready('plugins', function () {
    $(function () {
      const favsTippy = tippy(document.getElementById('favs_button'), {
        content: "Для добавления в избранное надо залогиниться!",
        trigger: 'manual'
      });
      const memoriesTippy = tippy(document.getElementById('memories_button'), {
        content: "Для добавления в отслеживаемое надо залогиниться!",
        trigger: 'manual'
      });

      $("#favs_button").on("click", function (event) {
        event.preventDefault();
        event.stopPropagation();
        memoriesTippy.hide();
        favsTippy.show();
      });

      $("#memories_button").on("click", function (event) {
        event.preventDefault();
        event.stopPropagation();
        favsTippy.hide();
        memoriesTippy.show();
      });
    });
  });
}

function topic_memories_form_setup(memId, watch, msgid, csrf) {
  function memories_add(event) {
    event.preventDefault();

    $.ajax({
      url: "/memories.jsp",
      method: "POST",
      data: {msgid: msgid, add: "add", watch: event.data.watch, csrf: csrf}
    }).done(function (t) {
      form_setup(t['id'], event.data.watch);
      if (event.data.watch) {
        $('#memories_count').text(t['count']);
      } else {
        $('#favs_count').text(t['count']);
      }
    });
  }

  function memories_remove(event) {
    event.preventDefault();

    $.ajax({
      url: "/memories.jsp",
      method: "POST",
      data: {id: event.data.id, remove: "remove", csrf: csrf}
    }).done(function (t) {
      form_setup(0, event.data.watch);
      if (t >= 0) {
        if (event.data.watch) {
          $('#memories_count').text(t);
        } else {
          $('#favs_count').text(t);
        }
      }
    });
  }

  function form_setup(memId, watch) {
    let el;

    if (watch) {
      el = $('#memories_button');
    } else {
      el = $('#favs_button');
    }

    if (memId == 0) {
      el.removeClass('selected');
      el.attr('title', watch ? "Отслеживать" : "В избранное");

      el.off("click", memories_remove);
      el.on("click", {watch: watch}, memories_add);
    } else {
      el.addClass('selected');
      el.attr('title', watch ? "Не отслеживать" : "Удалить из избранного");

      el.off("click", memories_add);
      el.on("click", {watch: watch, id: memId}, memories_remove);
    }
  }

  $(function () {
    form_setup(memId, watch);
  });
}

function tag_memories_form_setup(tag, csrf_token) {
  $script.ready('plugins', function () {
    $(function () {
      const tagFavTippy = tippy(document.getElementById('tagFavNoth'), {
        content: "Для добавления в избранное надо залогиниться!",
        trigger: 'manual'
      });
      const tagIgnTippy = tippy(document.getElementById('tagIgnNoth'), {
        content: "Для добавления в список игнорирования надо залогиниться!",
        trigger: 'manual'
      });

      $("#tagFavNoth").on("click", function (event) {
        event.preventDefault();
        event.stopPropagation();
        tagFavTippy.show();
      });

      $("#tagIgnNoth").on("click", function (event) {
        event.preventDefault();
        event.stopPropagation();
        tagIgnTippy.show();
      });

      function tag_filter(event) {
        event.preventDefault();

        const data = {tagName: tag};

        const el = $('#tagFavAdd');
        const add = !el.hasClass("selected");

        if (add) {
          data['add'] = 'add';
        } else {
          data['del'] = 'del';
        }

        data['csrf'] = csrf_token;

        $.ajax({
          url: "/user-filter/favorite-tag",
          method: "POST",
          dataType: "json",
          data: data
        }).done(function (t) {
          if (t.error) {
            alert(t.error);
          } else {
            el.attr('title', add ? "Удалить из избранного" : "В избранное");

            $('#favsCount').text(t['count']);

            if (add) {
              el.addClass("selected");
            } else {
              el.removeClass("selected");
            }
          }
        });
      }

      $("#tagFavAdd").on("click", tag_filter);

      function tag_ignore(event) {
        event.preventDefault();

        const data = {tagName: tag};

        const el = $('#tagIgnore');
        const add = !el.hasClass("selected");

        if (add) {
          data['add'] = 'add';
        } else {
          data['del'] = 'del';
        }

        data['csrf'] = csrf_token;

        $.ajax({
          url: "/user-filter/ignore-tag",
          method: "POST",
          dataType: "json",
          data: data
        }).done(function (t) {
          if (t.error) {
            alert(t.error);
          } else {
            el.attr('title', add ? "Перестать игнорировать" : "Игнорировать");

            $('#ignoreCount').text(t['count']);

            if (add) {
              el.addClass("selected");
            } else {
              el.removeClass("selected");
            }
          }
        });
      }

      $("#tagIgnore").on("click", tag_ignore);
    });
  });
}