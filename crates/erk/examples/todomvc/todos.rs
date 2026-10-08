//! TodoMVC on Erk's Rust API (M4.5): the page is static HTML and CSS, and
//! the host keeps the list and changes the document to match it. Adding a
//! task builds its row in one batch of mutations; ticking, removing,
//! filtering and counting are single calls. The new task's text is typed
//! into a focusable element: Erk has no `<input>` before M5, so the host
//! builds the text from key events (M4 plan, decision 4).
//!
//! The example (`main.rs`) opens it in a window; `tests/todomvc.rs` drives
//! the same code without one.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use erk::{App, Context, Event, EventKind, Key, KeyState, Mutation, Node, Ref, Status};

pub const PAGE: &str = r#"<!DOCTYPE html>
<html lang="tr">
<head>
<style>
  body { margin: 0; background: #f5f5f5; color: #111827; font-family: "Noto Sans", sans-serif; font-size: 16px; }
  .todoapp { width: 460px; margin: 24px auto; }
  h1 { margin: 0 0 12px; text-align: center; font-size: 40px; font-weight: 300; color: #b83f45; }
  .panel { background: #ffffff; border: 1px solid #e5e7eb; border-radius: 6px; box-shadow: 0 2px 6px rgba(0, 0, 0, 0.12); }
  .new-todo { padding: 14px 16px; border-bottom: 1px solid #e5e7eb; min-height: 22px; }
  .new-todo:focus { outline: none; background: #fffbeb; }
  .new-todo.placeholder { color: #9ca3af; font-style: italic; }
  .todo-list { margin: 0; padding: 0; list-style: none; }
  .todo { display: flex; align-items: center; gap: 12px; padding: 10px 16px; border-bottom: 1px solid #f3f4f6; }
  .toggle { width: 22px; height: 22px; border: 2px solid #d1d5db; border-radius: 50%; cursor: pointer; }
  .completed .toggle { border-color: #10b981; background: #10b981; }
  .label { flex: 1; }
  .completed .label { color: #9ca3af; }
  .destroy { border: 0; background: transparent; color: #cc9a9a; font-size: 20px; cursor: pointer; padding: 0 4px; }
  .filter-active .completed, .filter-completed .todo:not(.completed) { display: none; }
  .footer { display: flex; align-items: center; justify-content: space-between; gap: 8px; padding: 10px 16px; color: #6b7280; font-size: 14px; }
  .todo-count, .clear-completed { white-space: nowrap; }
  .filters { display: flex; gap: 4px; margin: 0; padding: 0; list-style: none; }
  .filters a { display: inline-block; padding: 2px 8px; border: 1px solid transparent; border-radius: 4px; color: inherit; text-decoration: none; cursor: pointer; }
  .filters a.selected { border-color: #e8bdbd; }
  .clear-completed { border: 0; background: transparent; color: inherit; font-size: 14px; cursor: pointer; }
  [hidden] { display: none; }
</style>
</head>
<body>
  <section class="todoapp">
    <h1>görevler</h1>
    <div class="panel">
      <div class="new-todo placeholder" tabindex="0">Ne yapılacak?</div>
      <ul class="todo-list"></ul>
      <footer class="footer" hidden>
        <span class="todo-count"></span>
        <ul class="filters">
          <li><a class="selected" id="all">Tümü</a></li>
          <li><a id="active">Yapılacak</a></li>
          <li><a id="completed">Tamamlanan</a></li>
        </ul>
        <button class="clear-completed" hidden>Temizle</button>
      </footer>
    </div>
  </section>
</body>
</html>"#;

/// What the new-task field shows while nothing is typed.
const PLACEHOLDER: &str = "Ne yapılacak?";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    All,
    Active,
    Completed,
}

/// A task and the row that shows it.
pub struct Todo {
    pub title: String,
    pub done: bool,
    pub row: Node,
}

/// The page's fixed parts.
struct Parts {
    field: Node,
    list: Node,
    count: Node,
    footer: Node,
    clear: Node,
    filters: [(Filter, Node); 3],
}

pub struct Todos {
    pub todos: Vec<Todo>,
    pub draft: String,
    pub filter: Filter,
    parts: Parts,
    /// The list itself, for the subscriptions of the rows it adds.
    me: Weak<RefCell<Todos>>,
}

pub type Shared = Rc<RefCell<Todos>>;

fn find(app: &App, selector: &str) -> Result<Node, Status> {
    app.query(None, selector)?.ok_or(Status::NotFound)
}

/// Load the page into `app` and wire it up.
pub fn mount(app: &mut App) -> Result<Shared, Status> {
    app.load_html(PAGE);
    let parts = Parts {
        field: find(app, ".new-todo")?,
        list: find(app, ".todo-list")?,
        count: find(app, ".todo-count")?,
        footer: find(app, ".footer")?,
        clear: find(app, ".clear-completed")?,
        filters: [
            (Filter::All, find(app, "#all")?),
            (Filter::Active, find(app, "#active")?),
            (Filter::Completed, find(app, "#completed")?),
        ],
    };
    let (field, clear, filters) = (parts.field, parts.clear, parts.filters);
    let todos: Shared = Rc::new_cyclic(|me| {
        RefCell::new(Todos {
            todos: Vec::new(),
            draft: String::new(),
            filter: Filter::All,
            parts,
            me: me.clone(),
        })
    });

    let shared = todos.clone();
    app.on(field, EventKind::KeyDown, move |cx, event| {
        shared.borrow_mut().key(cx, event);
    })?;
    let shared = todos.clone();
    app.on(clear, EventKind::Click, move |cx, _| {
        shared.borrow_mut().clear_completed(cx);
    })?;
    for (filter, link) in filters {
        let shared = todos.clone();
        app.on(link, EventKind::Click, move |cx, _| {
            shared.borrow_mut().show(cx, filter);
        })?;
    }
    Ok(todos)
}

impl Todos {
    /// A key typed into the new-task field.
    fn key(&mut self, cx: &mut Context, event: &Event) {
        let Some(key) = &event.key else {
            return;
        };
        match key {
            Key::Character(typed) if !event.modifiers.control && !event.modifiers.meta => {
                self.draft.push_str(typed);
            }
            Key::Space => self.draft.push(' '),
            Key::Backspace => {
                self.draft.pop();
            }
            Key::Escape => self.draft.clear(),
            Key::Enter => {
                let title = self.draft.trim().to_owned();
                if !title.is_empty() {
                    self.draft.clear();
                    // A failure leaves the list as it was.
                    let _ = self.add(cx, title);
                }
            }
            _ => return,
        }
        self.show_draft(cx);
    }

    fn show_draft(&self, cx: &mut Context) {
        let field = self.parts.field;
        let _ = if self.draft.is_empty() {
            cx.add_class(field, "placeholder")
                .and_then(|()| cx.set_text(field, PLACEHOLDER))
        } else {
            cx.remove_class(field, "placeholder")
                .and_then(|()| cx.set_text(field, &self.draft))
        };
    }

    /// Add a task: its row is built in one batch of mutations.
    fn add(&mut self, cx: &mut Context, title: String) -> Result<(), Status> {
        let made = cx
            .apply(&[
                Mutation::CreateElement("li".to_owned()),
                Mutation::AddClass(Ref::New(0), "todo".to_owned()),
                Mutation::CreateElement("div".to_owned()),
                Mutation::AddClass(Ref::New(2), "toggle".to_owned()),
                Mutation::CreateElement("span".to_owned()),
                Mutation::AddClass(Ref::New(4), "label".to_owned()),
                Mutation::SetText(Ref::New(4), title.clone()),
                Mutation::CreateElement("button".to_owned()),
                Mutation::AddClass(Ref::New(7), "destroy".to_owned()),
                Mutation::SetText(Ref::New(7), "×".to_owned()),
                Mutation::Append {
                    parent: Ref::New(0),
                    child: Ref::New(2),
                },
                Mutation::Append {
                    parent: Ref::New(0),
                    child: Ref::New(4),
                },
                Mutation::Append {
                    parent: Ref::New(0),
                    child: Ref::New(7),
                },
                Mutation::Append {
                    parent: Ref::Node(self.parts.list),
                    child: Ref::New(0),
                },
            ])
            .map_err(|failed| failed.status)?;
        let (Some(row), Some(toggle), Some(destroy)) = (made[0], made[2], made[7]) else {
            return Err(Status::InvalidArgument);
        };
        self.todos.push(Todo {
            title,
            done: false,
            row,
        });
        // The rows' own subscriptions end with them: removing a row is all
        // a removal takes.
        let shared = self.me.clone();
        let on_toggle = shared.clone();
        cx.on(toggle, EventKind::Click, move |cx, _| {
            if let Some(todos) = on_toggle.upgrade() {
                todos.borrow_mut().toggle(cx, row);
            }
        })?;
        cx.on(destroy, EventKind::Click, move |cx, _| {
            if let Some(todos) = shared.upgrade() {
                todos.borrow_mut().destroy(cx, row);
            }
        })?;
        self.show_counts(cx);
        Ok(())
    }

    fn toggle(&mut self, cx: &mut Context, row: Node) {
        let Some(todo) = self.todos.iter_mut().find(|todo| todo.row == row) else {
            return;
        };
        todo.done = !todo.done;
        let _ = if todo.done {
            cx.add_class(row, "completed")
        } else {
            cx.remove_class(row, "completed")
        };
        self.show_counts(cx);
    }

    fn destroy(&mut self, cx: &mut Context, row: Node) {
        self.todos.retain(|todo| todo.row != row);
        let _ = cx.remove(row);
        self.show_counts(cx);
    }

    fn clear_completed(&mut self, cx: &mut Context) {
        for todo in self.todos.iter().filter(|todo| todo.done) {
            let _ = cx.remove(todo.row);
        }
        self.todos.retain(|todo| !todo.done);
        self.show_counts(cx);
    }

    fn show(&mut self, cx: &mut Context, filter: Filter) {
        self.filter = filter;
        let list = self.parts.list;
        for (class, which) in [
            ("filter-active", Filter::Active),
            ("filter-completed", Filter::Completed),
        ] {
            let _ = if filter == which {
                cx.add_class(list, class)
            } else {
                cx.remove_class(list, class)
            };
        }
        for (which, link) in self.parts.filters {
            let _ = if filter == which {
                cx.add_class(link, "selected")
            } else {
                cx.remove_class(link, "selected")
            };
        }
    }

    /// The counter, and the footer and its button only when they apply.
    fn show_counts(&self, cx: &mut Context) {
        let left = self.todos.iter().filter(|todo| !todo.done).count();
        let _ = cx.set_text(self.parts.count, &format!("{left} görev kaldı"));
        let shown = |cx: &mut Context, node: Node, shown: bool| {
            let _ = if shown {
                cx.remove_attr(node, "hidden").map(drop)
            } else {
                cx.set_attr(node, "hidden", "")
            };
        };
        shown(cx, self.parts.footer, !self.todos.is_empty());
        shown(
            cx,
            self.parts.clear,
            self.todos.iter().any(|todo| todo.done),
        );
    }
}

// Unused in the example binary, used by the test through `#[path]`.
#[allow(dead_code)]
pub fn titles(todos: &Shared) -> Vec<String> {
    todos
        .borrow()
        .todos
        .iter()
        .map(|todo| todo.title.clone())
        .collect()
}

#[allow(dead_code)]
pub fn key_down(key: Key) -> erk::Input {
    erk::Input::Key(erk::KeyInput {
        key,
        state: KeyState::Down,
        modifiers: erk::Modifiers::default(),
    })
}
