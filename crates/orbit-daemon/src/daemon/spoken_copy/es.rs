use super::{PresenceVocabulary, SpokenCopy};

pub(super) const COPY: SpokenCopy = SpokenCopy {
    call_question: "¿Sigues ahí? Si no dices nada, cuelgo en 15 segundos",
    answers: PresenceVocabulary {
        keep: [
            &["sí", "si"],
            &["sigo"],
            &["aquí", "aqui"],
            &["espera"],
            &["mantén", "manten"],
            &["no cuelgues"],
        ],
        hang_up: &["cuelga"],
    },
};
