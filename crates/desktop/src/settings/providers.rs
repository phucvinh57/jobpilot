//! Settings > AI providers (P1): add/edit provider, test, choose the default.
//!
//! Views only: persistence, keyring and HTTP live in `jobpilot_core::llm` / `Core`.
//! Anything that can block (keyring, network) runs through `runtime::spawn`.

use std::future::Future;

use anyhow::anyhow;
use gpui_kit::component::button::*;
use gpui_kit::component::input::*;
use gpui_kit::component::select::*;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jobpilot_core::store::{Provider, ProviderKind};
use jobpilot_core::{llm, runtime};

use super::section;
use crate::app::AppState;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Busy {
    Idle,
    Saving,
    Testing,
    LoadingModels,
}

/// The provider being edited (`id == 0`: not saved yet).
struct Edit {
    id: i64,
    kind: ProviderKind,
    json_object: bool,
    is_default: bool,
    models: Vec<String>,
    confirm_delete: bool,
}

/// Input widgets; created on the first render because they need a `Window`.
struct Form {
    name: Entity<InputState>,
    base_url: Entity<InputState>,
    api_key: Entity<InputState>,
    model: Entity<InputState>,
    picker: Entity<SelectState<Vec<String>>>,
    _picker_sub: Subscription,
}

/// Inline result line under the buttons.
struct Notice {
    ok: bool,
    text: String,
}

pub struct ProvidersSection {
    state: Entity<AppState>,
    providers: Vec<Provider>,
    loaded: bool,
    form: Option<Form>,
    edit: Option<Edit>,
    busy: Busy,
    notice: Option<Notice>,
}

fn kind_label(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::OpenAi => "OpenAI-compatible",
        ProviderKind::Anthropic => "Anthropic",
    }
}

/// Name, base URL and model prefilled for a kind.
fn kind_defaults(kind: ProviderKind) -> (&'static str, &'static str, &'static str) {
    let name = match kind {
        ProviderKind::OpenAi => "OpenAI",
        ProviderKind::Anthropic => "Anthropic",
    };
    (name, llm::default_base_url(kind), llm::default_model(kind))
}

impl ProvidersSection {
    pub fn new(state: Entity<AppState>) -> Self {
        Self { state, providers: Vec::new(), loaded: false, form: None, edit: None, busy: Busy::Idle, notice: None }
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        match self.state.read(cx).core.store.list_providers() {
            Ok(list) => self.providers = list,
            Err(e) => tracing::error!("failed to load providers: {e:#}"),
        }
        self.loaded = true;
    }

    fn ensure_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.loaded {
            self.reload(cx);
        }
        if self.form.is_some() {
            return;
        }
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("Name"));
        let base_url = cx.new(|cx| InputState::new(window, cx).placeholder("Base URL"));
        let api_key =
            cx.new(|cx| InputState::new(window, cx).masked(true).placeholder("API key (optional for local servers)"));
        let model = cx.new(|cx| InputState::new(window, cx).placeholder("Model id, e.g. gpt-4o-mini"));
        let picker = cx.new(|cx| SelectState::new(Vec::<String>::new(), None, window, cx).searchable(true));
        let sub =
            cx.subscribe_in(&picker, window, |this: &mut Self, _, event: &SelectEvent<Vec<String>>, window, cx| {
                if let (SelectEvent::Confirm(Some(id)), Some(form)) = (event, &this.form) {
                    form.model.update(cx, |input, cx| input.set_value(id.clone(), window, cx));
                }
            });
        self.form = Some(Form { name, base_url, api_key, model, picker, _picker_sub: sub });
    }

    /// Show the form for `provider` (`None`: a new one).
    fn open(&mut self, provider: Option<Provider>, window: &mut Window, cx: &mut Context<Self>) {
        self.ensure_form(window, cx);
        let (id, kind, name, base_url, model, json_object, is_default) = match &provider {
            Some(p) => (
                p.id,
                p.kind,
                p.name.clone(),
                p.base_url.clone(),
                p.model.clone(),
                p.json_mode == llm::openai::JSON_MODE_OBJECT,
                p.is_default,
            ),
            None => {
                let (n, b, m) = kind_defaults(ProviderKind::OpenAi);
                (0, ProviderKind::OpenAi, n.into(), b.into(), m.into(), false, false)
            }
        };
        if let Some(form) = &self.form {
            form.name.update(cx, |i, cx| i.set_value(name, window, cx));
            form.base_url.update(cx, |i, cx| i.set_value(base_url, window, cx));
            form.model.update(cx, |i, cx| i.set_value(model, window, cx));
            form.api_key.update(cx, |i, cx| i.set_value("", window, cx));
            form.picker.update(cx, |s, cx| s.set_items(Vec::new(), window, cx));
        }
        self.edit = Some(Edit { id, kind, json_object, is_default, models: Vec::new(), confirm_delete: false });
        self.notice = None;
        cx.notify();
    }

    /// Switch kind; replaces name/URL/model only when they still hold the old kind's defaults.
    fn set_kind(&mut self, kind: ProviderKind, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(edit), Some(form)) = (self.edit.as_mut(), self.form.as_ref()) else { return };
        if edit.kind == kind {
            return;
        }
        let (old_name, old_url, old_model) = kind_defaults(edit.kind);
        let (new_name, new_url, new_model) = kind_defaults(kind);
        for (input, old, new) in
            [(&form.name, old_name, new_name), (&form.base_url, old_url, new_url), (&form.model, old_model, new_model)]
        {
            let current = input.read(cx).value();
            if current.trim().is_empty() || current.trim() == old {
                input.update(cx, |i, cx| i.set_value(new, window, cx));
            }
        }
        edit.kind = kind;
        edit.models.clear();
        form.picker.update(cx, |s, cx| s.set_items(Vec::new(), window, cx));
        self.notice = None;
        cx.notify();
    }

    /// Current form contents as a provider plus the key typed in (if any).
    fn draft(&self, cx: &App) -> Option<(Provider, Option<String>)> {
        let (edit, form) = (self.edit.as_ref()?, self.form.as_ref()?);
        let text = |input: &Entity<InputState>| input.read(cx).value().trim().to_string();
        let mut base_url = text(&form.base_url);
        if base_url.is_empty() {
            base_url = llm::default_base_url(edit.kind).to_string();
        }
        let key = text(&form.api_key);
        let provider = Provider {
            id: edit.id,
            name: text(&form.name),
            kind: edit.kind,
            base_url,
            model: text(&form.model),
            json_mode: if edit.json_object && edit.kind == ProviderKind::OpenAi {
                llm::openai::JSON_MODE_OBJECT.to_string()
            } else {
                "json_schema".to_string()
            },
            is_default: edit.is_default,
        };
        Some((provider, (!key.is_empty()).then_some(key)))
    }

    fn fail(&mut self, text: impl Into<String>) {
        self.notice = Some(Notice { ok: false, text: text.into() });
    }

    /// Run `work` on the core runtime, then call `done` on the UI thread.
    fn run<T, W, D>(&mut self, busy: Busy, window: &mut Window, cx: &mut Context<Self>, work: W, done: D)
    where
        T: Send + 'static,
        W: Future<Output = anyhow::Result<T>> + Send + 'static,
        D: FnOnce(&mut Self, anyhow::Result<T>, &mut Window, &mut Context<Self>) + 'static,
    {
        self.busy = busy;
        self.notice = None;
        cx.notify();
        let rx = runtime::spawn(work);
        cx.spawn_in(window, async move |this, cx| {
            let result = rx.await.unwrap_or_else(|_| Err(anyhow!("the background task was cancelled")));
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = Busy::Idle;
                done(this, result, window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((provider, key)) = self.draft(cx) else { return };
        if provider.name.is_empty() {
            self.fail("Enter a name.");
            cx.notify();
            return;
        }
        let core = self.state.read(cx).core.clone();
        self.run(
            Busy::Saving,
            window,
            cx,
            async move { core.save_provider(&provider, key.as_deref()) },
            |this, result, window, cx| match result {
                Ok(saved) => {
                    this.reload(cx);
                    if let Some(edit) = this.edit.as_mut() {
                        edit.id = saved.id;
                        edit.is_default = saved.is_default;
                    }
                    if let Some(form) = &this.form {
                        form.api_key.update(cx, |i, cx| i.set_value("", window, cx));
                    }
                    this.notice = Some(Notice { ok: true, text: "Saved.".into() });
                }
                Err(e) => this.fail(format!("Could not save: {e:#}")),
            },
        );
    }

    fn test(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((provider, typed_key)) = self.draft(cx) else { return };
        let core = self.state.read(cx).core.clone();
        self.run(
            Busy::Testing,
            window,
            cx,
            async move {
                let key = match typed_key {
                    Some(k) => Some(k),
                    None if provider.id != 0 => core.provider_api_key(provider.id)?,
                    None => None,
                };
                let client = llm::from_provider(&provider, key);
                llm::test_llm(client.as_ref()).await?;
                Ok(())
            },
            |this, result, _, _| {
                this.notice = Some(match result {
                    Ok(()) => Notice { ok: true, text: "\u{2713} Works".into() },
                    Err(e) => Notice { ok: false, text: format!("\u{2717} {e:#}") },
                });
            },
        );
    }

    fn load_models(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((provider, typed_key)) = self.draft(cx) else { return };
        let core = self.state.read(cx).core.clone();
        self.run(
            Busy::LoadingModels,
            window,
            cx,
            async move {
                let key = match typed_key {
                    Some(k) => Some(k),
                    None if provider.id != 0 => core.provider_api_key(provider.id)?,
                    None => None,
                };
                llm::from_provider(&provider, key).list_models().await
            },
            |this, result, window, cx| match result {
                Ok(models) => {
                    let text = format!("{} model(s) available; pick one from the list.", models.len());
                    if let Some(form) = &this.form {
                        form.picker.update(cx, |s, cx| s.set_items(models.clone(), window, cx));
                    }
                    if let Some(edit) = this.edit.as_mut() {
                        edit.models = models;
                    }
                    this.notice = Some(Notice { ok: true, text });
                }
                Err(e) => this.fail(format!("Could not load models: {e:#}")),
            },
        );
    }

    fn make_default(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.edit.as_ref().map(|e| e.id).filter(|id| *id != 0) else { return };
        match self.state.read(cx).core.store.set_default_provider(id) {
            Ok(()) => {
                if let Some(edit) = self.edit.as_mut() {
                    edit.is_default = true;
                }
                self.reload(cx);
                self.notice = Some(Notice { ok: true, text: "This is now the default provider.".into() });
            }
            Err(e) => self.fail(format!("{e:#}")),
        }
        cx.notify();
    }

    /// Two-step: the first click arms the button, the second deletes.
    fn delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.edit.as_mut() else { return };
        if edit.id == 0 {
            self.edit = None;
            cx.notify();
            return;
        }
        if !edit.confirm_delete {
            edit.confirm_delete = true;
            cx.notify();
            return;
        }
        let (id, core) = (edit.id, self.state.read(cx).core.clone());
        self.run(
            Busy::Saving,
            window,
            cx,
            async move { core.remove_provider(id) },
            |this, result, _, cx| match result {
                Ok(()) => {
                    this.edit = None;
                    this.reload(cx);
                }
                Err(e) => this.fail(format!("Could not delete: {e:#}")),
            },
        );
    }

    fn field(label: &str, muted: Hsla, input: impl IntoElement) -> Div {
        div().v_flex().gap_1().child(div().text_sm().text_color(muted).child(label.to_string())).child(input)
    }

    fn render_form(&self, edit: &Edit, form: &Form, muted: Hsla, ok: Hsla, bad: Hsla, cx: &mut Context<Self>) -> Div {
        let idle = self.busy == Busy::Idle;
        let kind_button = |id: &'static str, kind: ProviderKind| {
            Button::new(id)
                .label(kind_label(kind))
                .selected(edit.kind == kind)
                .on_click(cx.listener(move |this, _, window, cx| this.set_kind(kind, window, cx)))
        };

        let model_row =
            div().h_flex().gap_2().items_center().child(div().flex_1().min_w_0().child(Input::new(&form.model))).child(
                Button::new("load-models")
                    .label("Load models")
                    .loading(self.busy == Busy::LoadingModels)
                    .disabled(!idle)
                    .on_click(cx.listener(|this, _, window, cx| this.load_models(window, cx))),
            );

        let mut model_field = Self::field("Model", muted, model_row);
        if !edit.models.is_empty() {
            model_field = model_field.child(Select::new(&form.picker).placeholder("Pick from the loaded models"));
        }

        let delete_label = if edit.id == 0 {
            "Discard"
        } else if edit.confirm_delete {
            "Click again to delete"
        } else {
            "Delete"
        };

        div()
            .v_flex()
            .gap_3()
            .pt_2()
            .child(Self::field(
                "Type",
                muted,
                div()
                    .h_flex()
                    .gap_2()
                    .child(kind_button("kind-openai", ProviderKind::OpenAi))
                    .child(kind_button("kind-anthropic", ProviderKind::Anthropic)),
            ))
            .child(Self::field("Name", muted, Input::new(&form.name)))
            .child(Self::field("Base URL", muted, Input::new(&form.base_url)))
            .child(Self::field(
                if edit.id == 0 { "API key" } else { "API key (leave empty to keep the saved one)" },
                muted,
                Input::new(&form.api_key).mask_toggle(),
            ))
            .child(model_field)
            .when(edit.kind == ProviderKind::OpenAi, |d| {
                d.child(
                    Switch::new("json-object-mode")
                        .checked(edit.json_object)
                        .label("Endpoint lacks json_schema support: use json_object mode")
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            if let Some(edit) = this.edit.as_mut() {
                                edit.json_object = *checked;
                            }
                            cx.notify();
                        })),
                )
            })
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        Button::new("save-provider")
                            .primary()
                            .label("Save")
                            .loading(self.busy == Busy::Saving)
                            .disabled(!idle)
                            .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                    )
                    .child(
                        Button::new("test-provider")
                            .label("Test")
                            .loading(self.busy == Busy::Testing)
                            .disabled(!idle)
                            .on_click(cx.listener(|this, _, window, cx| this.test(window, cx))),
                    )
                    .when(edit.id != 0 && !edit.is_default, |d| {
                        d.child(
                            Button::new("default-provider")
                                .label("Make default")
                                .disabled(!idle)
                                .on_click(cx.listener(|this, _, _, cx| this.make_default(cx))),
                        )
                    })
                    .child(
                        Button::new("delete-provider")
                            .danger()
                            .outline()
                            .label(delete_label)
                            .disabled(!idle)
                            .on_click(cx.listener(|this, _, window, cx| this.delete(window, cx))),
                    )
                    .child(Button::new("close-provider").ghost().label("Close").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.edit = None;
                            this.notice = None;
                            cx.notify();
                        },
                    ))),
            )
            .when_some(self.notice.as_ref(), |d, n| {
                d.child(div().text_sm().min_w_0().text_color(if n.ok { ok } else { bad }).child(n.text.clone()))
            })
    }
}

impl Render for ProvidersSection {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_form(window, cx);
        let (muted, ok, bad, accent, star) = {
            let t = cx.theme();
            (t.muted_foreground, t.success, t.danger, t.accent, t.warning)
        };

        let editing_id = self.edit.as_ref().map(|e| e.id);
        let rows: Vec<_> = self
            .providers
            .iter()
            .map(|p| {
                let id = p.id;
                div()
                    .id(("provider", id as usize))
                    .h_flex()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .cursor_pointer()
                    .when(editing_id == Some(id), |d| d.bg(accent))
                    .hover(|d| d.bg(accent))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let provider = this.providers.iter().find(|p| p.id == id).cloned();
                        this.open(provider, window, cx);
                    }))
                    .child(div().w(px(16.)).flex_shrink_0().text_color(star).child(if p.is_default {
                        "\u{2605}"
                    } else {
                        ""
                    }))
                    .child(div().font_semibold().child(p.name.clone()))
                    .child(div().text_sm().text_color(muted).child(kind_label(p.kind)))
                    .child(div().flex_1().min_w_0().truncate().text_sm().text_color(muted).child(p.model.clone()))
            })
            .collect();

        let mut body = section(
            "AI providers",
            "OpenAI- or Anthropic-compatible endpoints; API keys are kept in the OS keyring. The \u{2605} provider is used for generation.",
            cx,
        );
        if rows.is_empty() {
            body = body.child(div().text_sm().text_color(muted).child("No provider yet. Add one to generate CVs."));
        } else {
            body = body.child(div().v_flex().gap_1().children(rows));
        }
        if self.edit.is_none() {
            body = body.child(
                div().h_flex().child(
                    Button::new("add-provider")
                        .label("Add provider")
                        .on_click(cx.listener(|this, _, window, cx| this.open(None, window, cx))),
                ),
            );
        }
        if let (Some(edit), Some(form)) = (self.edit.as_ref(), self.form.as_ref()) {
            let form_view = self.render_form(edit, form, muted, ok, bad, cx);
            body = body.child(form_view);
        }
        body
    }
}
