

use topcoat::{
    Result,
    view::{View, component, view},
};

struct Phase {
    number: &'static str,
    badge: &'static str,
    badge_class: &'static str,
    title: &'static str,
    body: &'static str,
}

const PHASES: &[Phase] = &[
    Phase {
        number: "01",
        badge: "In Progress",
        badge_class: "px-2 py-0.5 rounded bg-primary-fixed text-on-primary-fixed-variant font-ui-caption text-ui-caption font-semibold",
        title: "Phase 1: Offline Desktop",
        body: "Native GPUI and Rust engine for macOS, Linux, and Windows. Local SQLite storage, rich prose typography, and instant timeline navigation.",
    },
    Phase {
        number: "02",
        badge: "Next",
        badge_class: "px-2 py-0.5 rounded bg-surface-container text-on-surface-variant font-ui-caption text-ui-caption",
        title: "Phase 2: Mobile Companions",
        body: "Lightweight companion apps for iOS and Android. Quick capture for audio voice memos, analog photo drops, and quick daily notes on the go.",
    },
    Phase {
        number: "03",
        badge: "Future",
        badge_class: "px-2 py-0.5 rounded bg-surface-container text-on-surface-variant font-ui-caption text-ui-caption",
        title: "Phase 3: Encrypted Sync",
        body: "Optional zero-knowledge, end-to-end encrypted peer relay syncing between personal devices. No servers ever read your plain text.",
    },
];

#[component]
pub async fn roadmap() -> Result<impl View> {
    Ok(view! {
        <section class="w-full bg-surface-container-low py-20 sm:py-24" id="roadmap">
            <div class="max-w-6xl mx-auto px-6 lg:px-12 space-y-12">
                <div class="max-w-xl space-y-2">
                    <span class="font-ui-caption text-ui-caption text-primary uppercase tracking-widest font-semibold">"Roadmap"</span>
                    <h2 class="font-headline-lg text-headline-lg text-on-surface">"Phases of Development"</h2>
                    <p class="font-journal-body text-journal-body text-on-surface-variant">"A deliberate, unhurried trajectory towards complete digital sovereignty."</p>
                </div>
                <div class="grid grid-cols-1 md:grid-cols-3 gap-6">
                    for phase in PHASES {
                        <div class="p-6 rounded-xl bg-surface-container-lowest shadow-sm space-y-4">
                            <div class="flex items-center justify-between">
                                <span class="font-display-date text-display-date-mobile text-secondary-container leading-none">(phase.number)</span>
                                <span class=(phase.badge_class)>(phase.badge)</span>
                            </div>
                            <h3 class="font-headline-md text-headline-md text-on-surface">(phase.title)</h3>
                            <p class="font-journal-body text-journal-body text-on-surface-variant leading-relaxed">(phase.body)</p>
                        </div>
                    }
                </div>
            </div>
        </section>
    })
}
