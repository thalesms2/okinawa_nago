//! Formulário genérico: campos de texto, seleção e alternância, com erro por campo.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FieldId {
    Quality,
    Codec,
    Gpu,
    Verify,
    AudioBitrate,
    Force,
    Format,
    AudioTrack,
    Compat,
    Start,
    End,
    Exact,
    Output,
    Overwrite,
}

/// Que caracteres um campo de texto aceita.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextKind {
    Any,
    Digits,
    /// Dígitos, `:` e `.`.
    Time,
}

impl TextKind {
    fn accepts(self, c: char) -> bool {
        match self {
            TextKind::Any => !c.is_control(),
            TextKind::Digits => c.is_ascii_digit(),
            TextKind::Time => c.is_ascii_digit() || c == ':' || c == '.',
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldKind {
    Text {
        value: String,
        kind: TextKind,
        placeholder: &'static str,
    },
    Choice {
        options: Vec<&'static str>,
        idx: usize,
    },
    Toggle(bool),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub id: FieldId,
    pub label: &'static str,
    pub kind: FieldKind,
    pub error: Option<String>,
}

impl Field {
    pub fn text(
        id: FieldId,
        label: &'static str,
        kind: TextKind,
        placeholder: &'static str,
    ) -> Field {
        Field {
            id,
            label,
            kind: FieldKind::Text {
                value: String::new(),
                kind,
                placeholder,
            },
            error: None,
        }
    }

    pub fn choice(
        id: FieldId,
        label: &'static str,
        options: Vec<&'static str>,
        idx: usize,
    ) -> Field {
        Field {
            id,
            label,
            kind: FieldKind::Choice { options, idx },
            error: None,
        }
    }

    pub fn toggle(id: FieldId, label: &'static str, on: bool) -> Field {
        Field {
            id,
            label,
            kind: FieldKind::Toggle(on),
            error: None,
        }
    }
}

/// O que uma tecla fez no formulário.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormEvent {
    /// Nada mudou (tecla não tratada pelo formulário).
    Ignored,
    /// Só o foco mudou.
    Moved,
    /// O valor de um campo mudou.
    Changed(FieldId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Form {
    pub fields: Vec<Field>,
    pub focus: usize,
}

impl Form {
    pub fn new(fields: Vec<Field>) -> Form {
        Form { fields, focus: 0 }
    }

    pub fn focused(&self) -> &Field {
        &self.fields[self.focus]
    }

    pub fn focus_on(&mut self, id: FieldId) {
        if let Some(i) = self.fields.iter().position(|f| f.id == id) {
            self.focus = i;
        }
    }

    fn get(&self, id: FieldId) -> &Field {
        self.fields
            .iter()
            .find(|f| f.id == id)
            .expect("campo existe no formulário")
    }

    fn get_mut(&mut self, id: FieldId) -> &mut Field {
        self.fields
            .iter_mut()
            .find(|f| f.id == id)
            .expect("campo existe no formulário")
    }

    pub fn has(&self, id: FieldId) -> bool {
        self.fields.iter().any(|f| f.id == id)
    }

    pub fn text(&self, id: FieldId) -> &str {
        match &self.get(id).kind {
            FieldKind::Text { value, .. } => value.trim(),
            _ => "",
        }
    }

    pub fn set_text(&mut self, id: FieldId, text: String) {
        if let FieldKind::Text { value, .. } = &mut self.get_mut(id).kind {
            *value = text;
        }
    }

    pub fn choice(&self, id: FieldId) -> &'static str {
        match &self.get(id).kind {
            FieldKind::Choice { options, idx } => options[*idx],
            _ => "",
        }
    }

    pub fn toggle(&self, id: FieldId) -> bool {
        matches!(self.get(id).kind, FieldKind::Toggle(true))
    }

    pub fn set_error(&mut self, id: FieldId, error: Option<String>) {
        self.get_mut(id).error = error;
    }

    pub fn clear_errors(&mut self) {
        for f in &mut self.fields {
            f.error = None;
        }
    }

    pub fn first_error(&self) -> Option<(FieldId, &str)> {
        self.fields
            .iter()
            .find_map(|f| f.error.as_deref().map(|e| (f.id, e)))
    }

    /// Navegação e edição. Teclas que o formulário não usa voltam como `Ignored`.
    pub fn handle_key(&mut self, key: KeyEvent) -> FormEvent {
        let n = self.fields.len();
        match key.code {
            KeyCode::Down | KeyCode::Tab => {
                self.focus = (self.focus + 1) % n;
                return FormEvent::Moved;
            }
            KeyCode::Up | KeyCode::BackTab => {
                self.focus = (self.focus + n - 1) % n;
                return FormEvent::Moved;
            }
            _ => {}
        }
        let field = &mut self.fields[self.focus];
        let id = field.id;
        let changed = match (&mut field.kind, key.code) {
            (FieldKind::Choice { options, idx }, KeyCode::Right | KeyCode::Char(' ')) => {
                *idx = (*idx + 1) % options.len();
                true
            }
            (FieldKind::Choice { options, idx }, KeyCode::Left) => {
                *idx = (*idx + options.len() - 1) % options.len();
                true
            }
            (FieldKind::Toggle(on), KeyCode::Left | KeyCode::Right | KeyCode::Char(' ')) => {
                *on = !*on;
                true
            }
            (FieldKind::Text { value, .. }, KeyCode::Char('u'))
                if key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                let had = !value.is_empty();
                value.clear();
                had
            }
            (FieldKind::Text { value, .. }, KeyCode::Backspace) => value.pop().is_some(),
            (FieldKind::Text { value, kind, .. }, KeyCode::Char(c))
                if !key.modifiers.contains(KeyModifiers::CONTROL) && kind.accepts(c) =>
            {
                value.push(c);
                true
            }
            _ => false,
        };
        if changed {
            FormEvent::Changed(id)
        } else {
            FormEvent::Ignored
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn form() -> Form {
        Form::new(vec![
            Field::choice(FieldId::Quality, "qualidade", vec!["a", "b", "c"], 1),
            Field::toggle(FieldId::Gpu, "gpu", false),
            Field::text(FieldId::Start, "início", TextKind::Time, ""),
        ])
    }

    #[test]
    fn navegacao_circular() {
        let mut f = form();
        assert_eq!(f.handle_key(key(KeyCode::Up)), FormEvent::Moved);
        assert_eq!(f.focused().id, FieldId::Start);
        f.handle_key(key(KeyCode::Tab));
        assert_eq!(f.focused().id, FieldId::Quality);
    }

    #[test]
    fn edicao_de_cada_tipo() {
        let mut f = form();
        assert_eq!(
            f.handle_key(key(KeyCode::Left)),
            FormEvent::Changed(FieldId::Quality)
        );
        assert_eq!(f.choice(FieldId::Quality), "a");
        f.handle_key(key(KeyCode::Down));
        f.handle_key(key(KeyCode::Char(' ')));
        assert!(f.toggle(FieldId::Gpu));
        f.handle_key(key(KeyCode::Down));
        for c in "1:3x0.5".chars() {
            f.handle_key(key(KeyCode::Char(c)));
        }
        assert_eq!(
            f.text(FieldId::Start),
            "1:30.5",
            "x é recusado no campo de tempo"
        );
        f.handle_key(key(KeyCode::Backspace));
        assert_eq!(f.text(FieldId::Start), "1:30.");
        let ctrl_u = KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL);
        assert_eq!(f.handle_key(ctrl_u), FormEvent::Changed(FieldId::Start));
        assert_eq!(f.text(FieldId::Start), "");
        assert_eq!(f.handle_key(key(KeyCode::Char('q'))), FormEvent::Ignored);
    }
}
