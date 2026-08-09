document.addEventListener('DOMContentLoaded', function () {
  function readCsrfToken() {
    var tokenCookie = document.cookie.split('; ').find(function (cookie) {
      return cookie.startsWith('CSRF_TOKEN=');
    });
    if (!tokenCookie) return '';
    return tokenCookie.slice('CSRF_TOKEN='.length).replace(/(^")|("$)/g, '');
  }

  document.querySelectorAll('form').forEach(function (form) {
    var formatGroup = form.querySelector('[data-format-mode]');
    var textarea = formatGroup && formatGroup.querySelector('textarea');
    var previewButton = form.querySelector('button[name="preview"]');
    if (!formatGroup || !textarea || !previewButton) return;
    var nav = formatGroup.querySelector('.markup-tabs__nav');
    var panels = formatGroup.querySelector('.markup-tabs__content');
    var editorTab = nav.querySelector('[data-tab="editor"]');
    var editorPanel = panels.querySelector('[data-panel="editor"]');
    var previewTab = document.createElement('li');
    previewTab.className = 'markup-tabs__tab';
    previewTab.dataset.tab = 'preview';
    previewTab.textContent = 'Предпросмотр';
    var previewPanel = document.createElement('div');
    previewPanel.className = 'markup-tabs__panel';
    previewPanel.dataset.panel = 'preview';
    var previewContent = document.createElement('div');
    previewContent.className = 'markup-preview';
    previewPanel.appendChild(previewContent);
    nav.appendChild(previewTab);
    panels.appendChild(previewPanel);
    previewButton.hidden = true;

    async function showPreview() {
      previewContent.textContent = 'Загрузка…';
      var body = new URLSearchParams({text: textarea.value, markup: formatGroup.dataset.formatMode});
      var csrfToken = readCsrfToken();
      if (csrfToken) body.set('csrf', csrfToken);
      try {
        var response = await fetch('/markup/preview', {
          method: 'POST',
          headers: {'Content-Type': 'application/x-www-form-urlencoded'},
          body: body
        });
        var result = await response.json();
        if (!response.ok || result.error) throw new Error(result.error || response.statusText);
        previewContent.innerHTML = result.html || '';
      } catch (error) {
        previewContent.textContent = error.message;
      }
      editorTab.classList.remove('active');
      editorPanel.classList.remove('active');
      previewTab.classList.add('active');
      previewPanel.classList.add('active');
    }
    previewTab.addEventListener('click', showPreview);
    editorTab.addEventListener('click', function () {
      previewTab.classList.remove('active');
      previewPanel.classList.remove('active');
      editorTab.classList.add('active');
      editorPanel.classList.add('active');
      textarea.focus();
    });
  });

  var commentForm = document.getElementById('commentForm');
  if (!commentForm) return;
  var replyTo = commentForm.querySelector('input[name="replyto"]');
  var textarea = commentForm.querySelector('textarea[name="msg"]');
  var formContainer = commentForm.parentElement;
  document.querySelectorAll('a[href^="/add_comment.jsp"], a[href^="add_comment.jsp"], a[href^="/comment-message.jsp"], a[href^="comment-message.jsp"]').forEach(function (link) {
    link.addEventListener('click', function (event) {
      var url = new URL(link.href, window.location.href);
      var id = url.searchParams.get('replyto') || '0';
      if (!replyTo) return;
      event.preventDefault();
      replyTo.value = id;
      link.closest('.msg').after(formContainer);
      formContainer.style.display = 'block';
      textarea.focus();
    });
  });
  var cancel = commentForm.querySelector('#cancelButton');
  if (cancel) cancel.addEventListener('click', function () {
    replyTo.value = '0';
    textarea.value = '';
    if (formContainer.hasAttribute('style')) formContainer.style.display = 'none';
  });
  textarea.addEventListener('keydown', function (event) {
    if (event.key === 'Enter' && event.ctrlKey) {
      event.preventDefault();
      commentForm.requestSubmit(commentForm.querySelector('.btn-primary'));
    }
  });
  commentForm.addEventListener('submit', async function (event) {
    event.preventDefault();
    var submit = commentForm.querySelector('.btn-primary');
    submit.disabled = true;
    try {
      var response = await fetch('/add_comment_ajax', {
        method: 'POST',
        body: new URLSearchParams(new FormData(commentForm))
      });
      var result = await response.json();
      if (!response.ok || result.errors) {
        throw new Error(result.errors ? result.errors.join('\n') : response.statusText);
      }
      window.location.assign(result.url);
    } catch (error) {
      alert(error.message || 'Не удалось поместить комментарий');
      submit.disabled = false;
    }
  });
});
