//! Notas MIDI e editor de notas (piano roll).
//!
//! Estado autoritativo das notas: vive no core, a UI apenas
//! projeta e injeta edições. Nenhuma dependência de UI/Slint.

/// Uma nota MIDI do piano roll.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Note {
    pub id: i32,
    /// Altura MIDI (0..127, 60 = C4).
    pub pitch: i32,
    /// Posição inicial em beats.
    pub start: f32,
    /// Duração em beats.
    pub length: f32,
    /// Velocidade 0..127.
    pub velocity: i32,
}

/// Editor de notas: fonte de verdade das edições do piano roll.
#[derive(Debug, Default)]
pub struct NoteEditor {
    notes: Vec<Note>,
    next_id: i64,
}

impl NoteEditor {
    pub fn new() -> Self {
        Self {
            notes: Vec::new(),
            next_id: 1,
        }
    }

    /// Notas atuais (ordem de inserção).
    pub fn notes(&self) -> &[Note] {
        &self.notes
    }

    /// Adiciona uma nota e retorna o novo id.
    pub fn add(&mut self, pitch: i32, start: f32, length: f32, velocity: i32) -> i32 {
        let id = self.next_id as i32;
        self.notes.push(Note {
            id,
            pitch,
            start,
            length,
            velocity,
        });
        self.next_id += 1;
        id
    }

    /// Move a posição inicial de uma nota.
    pub fn move_note(&mut self, id: i32, start: f32) {
        if let Some(n) = self.notes.iter_mut().find(|n| n.id == id) {
            n.start = start;
        }
    }

    /// Redimensiona a duração de uma nota.
    pub fn resize_note(&mut self, id: i32, length: f32) {
        if let Some(n) = self.notes.iter_mut().find(|n| n.id == id) {
            n.length = length;
        }
    }

    /// Altera a velocidade (clamp 0..127).
    pub fn change_note_velocity(&mut self, id: i32, velocity: i32) {
        if let Some(n) = self.notes.iter_mut().find(|n| n.id == id) {
            n.velocity = velocity.clamp(0, 127);
        }
    }

    /// Remove uma nota.
    pub fn delete(&mut self, id: i32) {
        self.notes.retain(|n| n.id != id);
    }

    /// Remove todas as notas cujo id está na lista.
    pub fn delete_many(&mut self, ids: &[i32]) {
        self.notes.retain(|n| !ids.contains(&n.id));
    }

    /// Alinha start e length de TODAS as notas ao grid
    /// (1/denom beats).
    pub fn quantize(&mut self, denom: i32) {
        self.quantize_impl(denom, false, &[]);
    }

    /// Alinha start e length das notas selecionadas ao grid.
    pub fn quantize_selected(&mut self, denom: i32, selected_ids: &[i32]) {
        self.quantize_impl(denom, true, selected_ids);
    }

    fn quantize_impl(&mut self, denom: i32, selected_only: bool, selected_ids: &[i32]) {
        if denom <= 0 {
            return;
        }
        let step: f32 = 4.0 / denom as f32;

        for n in self.notes.iter_mut() {
            if selected_only && !selected_ids.contains(&n.id) {
                continue;
            }
            n.start = (n.start / step).round() * step;
            if n.start < 0.0 {
                n.start = 0.0;
            }
            if n.length <= 0.0 {
                n.length = step;
            } else {
                n.length = (n.length / step).round() * step;
                if n.length < step * 0.25 {
                    n.length = step;
                }
            }
        }
    }

    /// Notas que intersectam um retângulo (pixels da viewport).
    ///
    /// Retorna os ids na ordem de iteração (a primeira é a "primária"
    /// para a seleção). `last_note` é a nota do topo da grade.
    pub fn notes_in_rect(
        &self,
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        px_per_beat: f32,
        note_height: f32,
        last_note: i32,
    ) -> Vec<i32> {
        let mut hits: Vec<i32> = Vec::new();
        for n in &self.notes {
            let nx1 = n.start * px_per_beat;
            let nx2 = (n.start + n.length) * px_per_beat;
            let ny1 = (last_note as f32 - n.pitch as f32) * note_height;
            let ny2 = ny1 + note_height;

            if !(x2 < nx1 || x1 > nx2 || y2 < ny1 || y1 > ny2) {
                hits.push(n.id);
            }
        }
        hits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor_with_notes() -> NoteEditor {
        let mut e = NoteEditor::new();
        e.add(60, 0.0, 1.0, 100);
        e.add(62, 1.0, 0.5, 80);
        e.add(64, 4.0, 2.0, 64);
        e
    }

    #[test]
    fn add_assigns_incremental_ids() {
        let mut e = NoteEditor::new();
        assert_eq!(e.add(60, 0.0, 1.0, 100), 1);
        assert_eq!(e.add(61, 2.0, 1.0, 90), 2);
        assert_eq!(e.notes().len(), 2);
    }

    #[test]
    fn move_and_resize_by_id() {
        let mut e = editor_with_notes();
        e.move_note(1, 3.5);
        e.resize_note(1, 2.0);
        let n = e.notes()[0];
        assert_eq!(n.start, 3.5);
        assert_eq!(n.length, 2.0);
    }

    #[test]
    fn velocity_is_clamped() {
        let mut e = editor_with_notes();
        e.change_note_velocity(2, 500);
        assert_eq!(e.notes()[1].velocity, 127);
        e.change_note_velocity(2, -5);
        assert_eq!(e.notes()[1].velocity, 0);
    }

    #[test]
    fn delete_and_delete_many() {
        let mut e = editor_with_notes();
        e.delete(1);
        assert_eq!(e.notes().len(), 2);
        e.delete_many(&[2, 3]);
        assert!(e.notes().is_empty());
    }

    #[test]
    fn quantize_aligns_all_notes() {
        let mut e = NoteEditor::new();
        e.add(60, 0.3, 0.4, 100);
        e.add(62, 1.9, 0.9, 100);
        e.quantize(16); // 1/16 de compasso = 0.25 beat
        let n0 = e.notes()[0];
        let n1 = e.notes()[1];
        assert_eq!(n0.start, 0.25);
        assert_eq!(n0.length, 0.5);
        assert_eq!(n1.start, 2.0);
        assert_eq!(n1.length, 1.0);
    }

    #[test]
    fn quantize_ignores_invalid_denominator() {
        let mut e = editor_with_notes();
        e.quantize(0);
        e.quantize(-4);
        assert_eq!(e.notes()[0].start, 0.0);
    }

    #[test]
    fn quantize_selected_only_touches_selection() {
        let mut e = NoteEditor::new();
        e.add(60, 0.3, 0.4, 100);
        e.add(62, 1.9, 0.9, 100);
        e.quantize_selected(16, &[1]);
        assert_eq!(e.notes()[0].start, 0.25);
        assert_eq!(e.notes()[1].start, 1.9);
    }

    #[test]
    fn quantize_denominator_is_bar_relative() {
        let mut e = NoteEditor::new();
        e.add(60, 0.3, 0.4, 100);
        e.quantize(4); // 1/4 de compasso = 1 beat
        let n = e.notes()[0];
        assert_eq!(n.start, 0.0);
        assert_eq!(n.length, 1.0);
    }

    #[test]
    fn notes_in_rect_hit_test() {
        let e = editor_with_notes();
        // Nota 1: start 0, length 1 → x 0..10 em px/beat=10; pitch 60,
        // last_note=64 → y (64-60)*20=80..100
        let hits = e.notes_in_rect(2.0, 85.0, 8.0, 95.0, 10.0, 20.0, 64);
        assert_eq!(hits, vec![1]);
        // Retângulo longe de tudo → vazio
        let none = e.notes_in_rect(100.0, 100.0, 120.0, 120.0, 10.0, 20.0, 64);
        assert!(none.is_empty());
    }
}