





use topcoat::{
    Result,
    router::href,
    view::{View, class, component, view},
};

use crate::{
    landing::WaitlistStatus,
    waitlist::join,
};

#[component]
pub async fn hero(status: Option<WaitlistStatus>) -> Result<impl View> {
    let (message, tone) = match status {
        Some(status) => (status.message(), status.tone_class()),
        None => ("", ""),
    };

    Ok(view! {
        <div class="relative w-full overflow-hidden">
            <div class="absolute -top-32 left-1/2 -translate-x-1/2 w-[840px] h-[340px] bg-gradient-to-b from-primary-fixed/20 via-surface-container-high/40 to-transparent blur-3xl pointer-events-none -z-10"></div>
            <section class="max-w-5xl mx-auto px-6 lg:px-12 pt-16 pb-16 sm:pt-24 sm:pb-20 text-center flex flex-col items-center" id="waitlist">
                <div class="inline-flex items-center gap-2 px-3.5 py-1.5 rounded-full bg-surface-container-high text-on-surface-variant font-ui-caption text-ui-caption shadow-sm mb-8 transition-transform hover:scale-[1.01]">
                    <span class="w-1.5 h-1.5 rounded-full bg-primary animate-pulse"></span>
                    <span class="font-ui-label text-ui-label tracking-wide uppercase">"Designed by SMBL"</span>
                    <span class="text-outline-variant">"•"</span>
                    <span>"Built in Rust & GPUI"</span>
                    <span class="text-outline-variant">"•"</span>
                    <span class="text-primary font-medium">"Zero Cloud Lock-in"</span>
                </div>
                <h1 class="font-headline-lg text-headline-lg sm:text-[3.25rem] sm:leading-[3.75rem] text-on-surface max-w-4xl tracking-tight">"A quiet, continuous river" <br class="hidden sm:inline"> "for your thoughts."</h1>
                <p class="font-journal-body-lg text-journal-body-lg text-on-surface-variant max-w-2xl mt-6 leading-relaxed">"Dainiki is a private, local-first daily journal crafted for those who value reflection over productivity metrics. No cards. No document switching. Just your life in an unbroken, beautifully typeset stream."</p>
                <form class="mt-10 w-full max-w-md flex flex-col sm:flex-row gap-2.5 items-stretch" method="post" action=(href!(join))>
                    <input class="flex-1 px-4 py-3 rounded bg-surface-container-low border border-surface-container text-on-surface font-ui-body placeholder:text-outline focus:outline-none focus:ring-1 focus:ring-primary shadow-sm" name="email" placeholder="Enter your email to join the private waitlist" type="email" required="">
                    <input aria-hidden="true" class="hidden" name="website" tabindex="-1" type="text">
                    <button class="px-6 py-3 rounded bg-primary text-on-primary font-ui-header text-ui-header shadow hover:bg-primary-container transition-all text-center whitespace-nowrap" type="submit">"Join Waitlist"</button>
                </form>
                if !message.is_empty() {
                    <p class=(class!("mt-4 font-ui-caption text-ui-caption max-w-lg", tone)) role="status">(message)</p>
                }
                <p class="mt-4 text-on-surface-variant font-ui-caption text-ui-caption max-w-lg">"Offline-first product coming soon to macOS, Windows, and Linux. Followed by iOS & Android, and optional paid sync."</p>
            </section>
        </div>
    })
}
