

use topcoat::{
    Result,
    router::href,
    view::{View, component, view},
};

use crate::landing::home;

#[component]
pub async fn header() -> Result<impl View> {
    Ok(view! {
        <header class="fixed top-0 left-0 right-0 z-50 bg-surface/80 backdrop-blur-xl shadow-[0_1px_8px_rgba(44,42,41,0.04)]">
            <div class="h-16 max-w-7xl mx-auto px-6 lg:px-12 flex items-center justify-between gap-gutter">
                <div class="flex items-center gap-space-lg">
                    <a class="flex items-center gap-space-sm group" href=(href!(home))>
                        <div class="w-8 h-8 rounded-lg bg-primary-container text-on-primary-container flex items-center justify-center transition-transform group-hover:scale-105">
                            <span class="material-symbols-outlined text-[18px]">"spa"</span>
                        </div>
                        <div class="flex flex-col">
                            <span class="font-headline-sm text-headline-sm tracking-tight text-on-surface leading-none">"Dainiki"</span>
                            <span class="font-ui-caption text-ui-caption text-on-surface-variant tracking-wider uppercase leading-none mt-0.5">"by SMBL"</span>
                        </div>
                    </a>
                </div>
                <nav class="hidden md:flex items-center gap-6">
                    <a class="font-ui-body text-ui-body text-on-surface-variant hover:text-on-surface transition-colors" href="#manifesto">"Manifesto"</a>
                    <a class="font-ui-body text-ui-body text-on-surface-variant hover:text-on-surface transition-colors" href="#roadmap">"Roadmap"</a>
                </nav>
                <div class="flex items-center gap-space-md">
                    <a class="inline-flex items-center px-4 py-2 rounded bg-primary text-on-primary font-ui-label text-ui-label hover:bg-primary-container hover:text-on-primary-container shadow-[0_1px_2px_rgba(44,42,41,0.08)] transition-all" href="#waitlist">"Join Waitlist"</a>
                </div>
            </div>
        </header>
    })
}
