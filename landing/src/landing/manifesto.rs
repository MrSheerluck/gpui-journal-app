

use topcoat::{
    Result,
    view::{View, component, view},
};

#[component]
pub async fn pull_quote() -> Result<impl View> {
    Ok(view! {
        <section class="max-w-4xl mx-auto px-6 lg:px-12 py-16 text-center" id="manifesto">
            <div class="space-y-6">
                <div class="w-8 h-8 mx-auto text-primary opacity-60">
                    <span class="material-symbols-outlined text-[32px]">"format_quote"</span>
                </div>
                <p class="font-headline-lg text-headline-lg sm:text-[2rem] sm:leading-[2.75rem] text-on-surface italic max-w-3xl mx-auto">"“Meaningful reflection does not emerge from forcing friction, but rather from aligning oneself with the grain of your own memory.”"</p>
                <div class="flex flex-col items-center gap-1">
                    <span class="font-ui-header text-ui-header text-on-surface">"The SMBL Journaling Treatise"</span>
                    <span class="font-ui-caption text-ui-caption text-on-surface-variant">"Kyoto • Volume IV"</span>
                </div>
            </div>
        </section>
    })
}
