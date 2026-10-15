

use topcoat::{
    Result,
    view::{View, component, view},
};

#[component]
pub async fn footer() -> Result<impl View> {
    Ok(view! {
        <footer class="w-full bg-surface-container-low shadow-[0_-1px_6px_rgba(44,42,41,0.02)]">
            <div class="max-w-7xl mx-auto px-6 lg:px-12 py-space-xl flex flex-col md:flex-row items-center justify-between gap-gutter">
                <div class="flex flex-col items-center md:items-start gap-space-xs">
                    <div class="flex items-center gap-space-sm">
                        <span class="font-headline-sm text-headline-sm text-on-surface">"Dainiki"</span>
                        <span class="font-ui-caption text-ui-caption text-on-surface-variant">"© 2026 SMBL Foundation. Open source under the MIT License."</span>
                    </div>
                    <p class="font-ui-caption text-ui-caption text-on-surface-variant">"Crafted for mindful documentarians and serene digital introspection."</p>
                </div>
                <div class="flex flex-wrap items-center justify-center gap-space-lg">
                    <a class="font-ui-caption text-ui-caption text-on-surface-variant hover:text-on-surface transition-colors" href="#manifesto">"Manifesto"</a>
                    <a class="font-ui-caption text-ui-caption text-on-surface-variant hover:text-on-surface transition-colors" href="#roadmap">"Roadmap"</a>
                    <a class="font-ui-caption text-ui-caption text-on-surface-variant hover:text-on-surface transition-colors" href="#waitlist">"Waitlist"</a>
                    <a class="font-ui-caption text-ui-caption text-on-surface-variant hover:text-on-surface transition-colors" href="https://github.com/MrSheerluck/smbl-dainiki" rel="noopener" target="_blank">"Source Code (GitHub)"</a>
                </div>
            </div>
        </footer>
    })
}
