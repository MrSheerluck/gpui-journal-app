

use topcoat::{
    Result,
    view::{View, class, component, view},
};

use topcoat::asset::asset;

struct Tag {
    name: &'static str,
    count: &'static str,
}

const TAGS: &[Tag] = &[
    Tag {
        name: "reflections",
        count: "42",
    },
    Tag {
        name: "travel",
        count: "18",
    },
    Tag {
        name: "morning",
        count: "64",
    },
    Tag {
        name: "work",
        count: "31",
    },
    Tag {
        name: "ideas",
        count: "15",
    },
];



const BARS: &[&str] = &[
    "w-0.5 h-2.5 bg-primary rounded-full",
    "w-0.5 h-4 bg-primary rounded-full",
    "w-0.5 h-5 bg-primary rounded-full",
    "w-0.5 h-3 bg-primary rounded-full",
    "w-0.5 h-5 bg-primary rounded-full",
    "w-0.5 h-4 bg-primary rounded-full",
    "w-0.5 h-3.5 bg-primary rounded-full",
    "w-0.5 h-5 bg-primary rounded-full",
    "w-0.5 h-6 bg-primary rounded-full",
    "w-0.5 h-3 bg-primary rounded-full",
    "w-0.5 h-4.5 bg-primary rounded-full",
    "w-0.5 h-3.5 bg-primary rounded-full",
    "w-0.5 h-5 bg-primary rounded-full",
    "w-0.5 h-4 bg-primary rounded-full",
    "w-0.5 h-3.5 bg-outline-variant/40 rounded-full",
    "w-0.5 h-5 bg-outline-variant/40 rounded-full",
    "w-0.5 h-3 bg-outline-variant/40 rounded-full",
    "w-0.5 h-5.5 bg-outline-variant/40 rounded-full",
    "w-0.5 h-4 bg-outline-variant/40 rounded-full",
    "w-0.5 h-2 bg-outline-variant/40 rounded-full",
    "w-0.5 h-5 bg-outline-variant/40 rounded-full",
    "w-0.5 h-3.5 bg-outline-variant/40 rounded-full",
    "w-0.5 h-1.5 bg-outline-variant/40 rounded-full",
];

#[component]
pub async fn showcase() -> Result<impl View> {
    Ok(view! {
        <section class="max-w-5xl mx-auto px-4 sm:px-6 lg:px-12 pb-24 w-full">
            <div class="rounded-xl bg-surface-container-lowest shadow-2xl p-2 sm:p-4">
                
                <div class="flex items-center justify-between px-3.5 py-2.5 bg-surface/90 border-b border-surface-container-high/60 rounded-t-lg select-none">
                    <div class="flex items-center gap-space-md">
                        <div class="flex items-center gap-1.5">
                            <div class="w-3 h-3 rounded-full bg-[#ec6a5e] shadow-inner"></div>
                            <div class="w-3 h-3 rounded-full bg-[#f4bf4f] shadow-inner"></div>
                            <div class="w-3 h-3 rounded-full bg-[#62c554] shadow-inner"></div>
                        </div>
                        <div class="flex items-center gap-space-sm pl-space-xs">
                            <img alt="Dainiki Logo" class="h-6 w-auto object-contain" src=(asset!("./img/logo.png"))>
                            <span class="font-ui-header text-ui-header font-semibold text-on-surface tracking-tight text-xs">"Dainiki"</span>
                        </div>
                    </div>
                    <div class="flex items-center gap-space-sm">
                        <div class="relative hidden sm:flex items-center">
                            <div class="pl-2.5 pr-2.5 py-1 text-[11px] bg-surface-container-low rounded-full border border-surface-container text-on-surface-variant/70 w-44 flex items-center justify-between whitespace-nowrap select-none">
                                <span class="text-[11px] text-on-surface-variant/80 truncate pr-2">"Search entries..."</span>
                                <span class="font-mono text-[9px] px-1.5 py-0.5 rounded bg-surface-container text-on-surface-variant/80 shrink-0 leading-none">"⌘K"</span>
                            </div>
                        </div>
                    </div>
                </div>

                
                <div class="flex flex-col md:flex-row w-full bg-surface rounded-b-lg overflow-hidden border border-t-0 border-surface-container">
                    
                    <aside class="hidden md:flex flex-col justify-between w-[220px] shrink-0 bg-surface-container-low/70 border-r border-surface-container-high/50 select-none text-left">
                        <div class="flex-1 flex flex-col pt-space-sm px-space-sm overflow-hidden">
                            <nav class="space-y-0.5">
                                <div class="flex items-center justify-between px-space-sm py-1 rounded bg-surface-container-highest text-on-surface font-medium text-xs">
                                    <div class="flex items-center gap-space-sm">
                                        <span class="material-symbols-outlined text-[16px] text-primary">"view_stream"</span>
                                        <span>"Timeline"</span>
                                    </div>
                                    <span class="w-1.5 h-1.5 rounded-full bg-primary"></span>
                                </div>
                                <div class="flex items-center justify-between px-space-sm py-1 rounded text-on-surface-variant hover:bg-surface-container-high text-xs transition-colors">
                                    <div class="flex items-center gap-space-sm">
                                        <span class="material-symbols-outlined text-[16px]">"search"</span>
                                        <span>"Search"</span>
                                    </div>
                                    <span class="font-ui-caption text-ui-caption px-1.5 py-0.5 rounded bg-surface-container text-on-surface-variant font-mono text-[9px]">"⌘K"</span>
                                </div>
                            </nav>
                            <div class="mt-space-md pt-space-xs">
                                <div class="px-space-sm pb-1 flex items-center justify-between text-on-surface-variant/80">
                                    <span class="font-ui-caption text-ui-caption uppercase tracking-wider font-semibold text-[10px]">"Tags"</span>
                                    <span class="material-symbols-outlined text-[13px] text-on-surface-variant/70">"add"</span>
                                </div>
                                <nav class="space-y-0.5">
                                    for tag in TAGS {
                                        <div class="flex items-center justify-between px-space-sm py-1 rounded text-on-surface-variant font-ui-label text-xs hover:bg-surface-container-high transition-colors">
                                            <span class="flex items-center gap-1.5">
                                                <span class="text-secondary/70 font-mono">"#"</span>
                                                (tag.name)
                                            </span>
                                            <span class="font-ui-caption text-ui-caption text-on-surface-variant/60 text-[10px]">(tag.count)</span>
                                        </div>
                                    }
                                </nav>
                            </div>
                        </div>
                        <div class="p-space-sm bg-surface-container-low/40 space-y-space-sm border-t border-surface-container-high/40">
                            <div class="bg-surface-container-lowest/80 rounded-xl p-space-xs shadow-[0_1px_2px_rgba(44,42,41,0.02)]">
                                <div class="flex items-center justify-between px-1 pb-1">
                                    <span class="font-ui-caption text-ui-caption font-semibold text-on-surface text-[10px]">"October 2025"</span>
                                    <div class="flex items-center gap-0.5 text-on-surface-variant">
                                        <span class="material-symbols-outlined text-[12px]">"chevron_left"</span>
                                        <span class="material-symbols-outlined text-[12px]">"chevron_right"</span>
                                    </div>
                                </div>
                                <div class="grid grid-cols-7 gap-1 text-center font-ui-caption text-ui-caption text-on-surface-variant text-[9px]">
                                    <span>"M"</span>
                                    <span>"T"</span>
                                    <span>"W"</span>
                                    <span>"T"</span>
                                    <span>"F"</span>
                                    <span>"S"</span>
                                    <span>"S"</span>
                                </div>
                                <div class="grid grid-cols-7 gap-1 text-center font-ui-caption text-ui-caption pt-1 text-[10px]">
                                    for day in 29..=30u8 {
                                        <span class="text-on-surface-variant/30">(day)</span>
                                    }
                                    for day in 1..=26u8 {
                                        <span class=(class!("font-semibold text-primary relative after:content-[''] after:absolute after:bottom-0 after:left-1/2 after:-translate-x-1/2 after:w-1 after:h-1 after:rounded-full after:bg-primary" if day == 15))>(day)</span>
                                    }
                                </div>
                            </div>
                            <div class="flex items-center justify-between px-space-xs text-xs">
                                <div class="flex items-center gap-space-sm text-on-surface-variant font-ui-label">
                                    <span class="material-symbols-outlined text-[14px]">"settings"</span>
                                    <span>"Settings"</span>
                                </div>
                                <span class="font-ui-caption text-ui-caption px-1.5 py-0.5 rounded bg-surface-container text-on-surface-variant/70 font-mono text-[9px]">"⌘,"</span>
                            </div>
                            <div class="flex items-center gap-1.5 px-space-xs text-on-surface-variant/60">
                                <span class="material-symbols-outlined text-[12px] text-primary">"lock_clock"</span>
                                <span class="font-ui-caption text-ui-caption text-[10px]">"Directly editable stream"</span>
                            </div>
                        </div>
                    </aside>

                    
                    <div class="flex-1 bg-surface p-4 sm:p-8 overflow-y-auto text-left relative">
                        <div class="max-w-[42rem] mx-auto relative">
                            <div class="absolute -left-7 top-6 bottom-6 w-px bg-outline-variant/20 hidden xl:block pointer-events-none"></div>

                            
                            <article class="relative mb-space-lg pt-1">
                                <div class="absolute -left-24 top-1 text-right hidden xl:block select-none opacity-40">
                                    <span class="block font-display-date text-display-date text-outline-variant leading-none">"15"</span>
                                    <span class="font-ui-caption text-ui-caption tracking-widest uppercase text-on-surface-variant text-[10px]">"TODAY"</span>
                                </div>
                                <header class="mb-space-sm">
                                    <div class="flex items-baseline justify-between gap-y-1 mb-0.5">
                                        <div class="flex items-center gap-2">
                                            <span class="inline-block w-2 h-2 rounded-full bg-primary animate-pulse"></span>
                                            <h2 class="font-headline-md text-headline-md text-on-surface tracking-tight font-normal">"Today, Thursday, October 15"</h2>
                                        </div>
                                        <span class="font-ui-caption text-ui-caption text-on-surface-variant/50 flex items-center gap-1">
                                            <span class="material-symbols-outlined text-[13px]">"edit_note"</span>
                                            "Click to write"
                                        </span>
                                    </div>
                                    <h3 class="font-headline-sm text-[1.1rem] text-secondary font-normal italic">"The slow return of morning fog"</h3>
                                </header>
                                <div class="space-y-space-sm font-journal-body text-[0.95rem] text-on-surface leading-relaxed">
                                    <p class="first-letter:font-journal-body-lg first-letter:text-[2.2rem] first-letter:leading-none first-letter:float-left first-letter:mr-2 first-letter:font-normal first-letter:text-primary">"Before the sun cleared the ridgeline behind Tenryu-ji, the entire gorge lay suspended under a pale river vapor. Walking through the Arashiyama bamboo path at this hour is an exercise in acoustic subtraction: only the steady, hollow knocking of towering culms fifty feet overhead. I stopped for roasted hojicha by the hearth, grateful for an unhurried morning."</p>
                                    
                                    <div class="my-space-sm grid grid-cols-2 gap-space-sm pt-space-xs">
                                        <figure class="relative overflow-hidden rounded-lg bg-surface-container shadow-sm aspect-[16/10]">
                                            <img alt="Golden sunlight through bamboo canopy" class="w-full h-full object-cover" src=(asset!("./img/bamboo.jpg"))>
                                        </figure>
                                        <figure class="relative overflow-hidden rounded-lg bg-surface-container shadow-sm aspect-[16/10]">
                                            <img alt="Steaming hojicha tea in ceramic cup" class="w-full h-full object-cover" src=(asset!("./img/hojicha.jpg"))>
                                        </figure>
                                    </div>
                                    <div class="pt-1 text-on-surface-variant/40 flex items-center gap-1.5">
                                        <span class="w-0.5 h-4 bg-primary/70 animate-pulse inline-block"></span>
                                        <span class="italic font-journal-body text-xs">"Add thoughts for today..."</span>
                                    </div>
                                </div>
                            </article>

                            
                            <div class="my-space-md flex items-center justify-center">
                                <div class="w-12 h-px bg-outline-variant/30"></div>
                            </div>

                            
                            <article class="relative mb-space-lg pt-1">
                                <div class="absolute -left-24 top-1 text-right hidden xl:block select-none opacity-40">
                                    <span class="block font-display-date text-display-date text-outline-variant leading-none">"14"</span>
                                    <span class="font-ui-caption text-ui-caption tracking-widest uppercase text-on-surface-variant text-[10px]">"WED"</span>
                                </div>
                                <header class="mb-space-sm">
                                    <div class="flex items-baseline justify-between gap-y-1 mb-0.5">
                                        <h2 class="font-headline-md text-headline-md text-on-surface tracking-tight font-normal">"Wednesday, October 14"</h2>
                                        <span class="font-ui-caption text-ui-caption text-on-surface-variant/50 flex items-center gap-1">
                                            <span class="material-symbols-outlined text-[13px]">"edit"</span>
                                            "Click to edit"
                                        </span>
                                    </div>
                                </header>
                                <div class="space-y-space-sm font-journal-body text-[0.95rem] text-on-surface leading-relaxed">
                                    <p>"Spent hours refining the native GPUI render loop. When text and layouts respond in a single 120Hz display frame, software stops feeling like a separate tool and starts feeling like an extension of thought."</p>
                                    
                                    <div class="my-space-xs bg-surface-container-lowest rounded-lg p-2.5 border border-surface-container-high/60 transition-all">
                                        <div class="flex items-center justify-between gap-space-sm mb-1.5">
                                            <div class="flex items-center gap-2 min-w-0">
                                                <button aria-label="Play recording" class="w-7 h-7 rounded-full bg-primary text-on-primary flex items-center justify-center shrink-0 shadow-sm" type="button">
                                                    <span class="material-symbols-outlined text-[15px]">"play_arrow"</span>
                                                </button>
                                                <span class="font-ui-header text-xs text-on-surface truncate font-medium">"Ambient rain on workshop zinc roof"</span>
                                            </div>
                                            <div class="font-mono text-[10px] text-on-surface-variant/70">"01:48 / 04:12"</div>
                                        </div>
                                        <div class="relative w-full h-6 flex items-center gap-[2px] px-1 py-0.5 bg-surface-container-low rounded overflow-hidden">
                                            <div class="absolute inset-y-0 left-0 bg-primary-fixed/30 pointer-events-none rounded-l" style="width: 45%;"></div>
                                            for bar in BARS {
                                                <div class=(bar)></div>
                                            }
                                        </div>
                                    </div>
                                </div>
                            </article>

                            
                            <div class="my-space-md flex items-center justify-center">
                                <div class="w-12 h-px bg-outline-variant/30"></div>
                            </div>

                            
                            <article class="relative pt-1">
                                <div class="absolute -left-24 top-1 text-right hidden xl:block select-none opacity-40">
                                    <span class="block font-display-date text-display-date text-outline-variant leading-none">"13"</span>
                                    <span class="font-ui-caption text-ui-caption tracking-widest uppercase text-on-surface-variant text-[10px]">"TUE"</span>
                                </div>
                                <header class="mb-space-sm">
                                    <div class="flex items-baseline justify-between gap-y-1 mb-0.5">
                                        <h2 class="font-headline-md text-headline-md text-on-surface tracking-tight font-normal">"Tuesday, October 13"</h2>
                                        <span class="font-ui-caption text-ui-caption text-on-surface-variant/50 flex items-center gap-1">
                                            <span class="material-symbols-outlined text-[13px]">"edit"</span>
                                            "Click to edit"
                                        </span>
                                    </div>
                                    <h3 class="font-headline-sm text-[1.1rem] text-secondary font-normal italic">"Architectural shadows in the studio"</h3>
                                </header>
                                <div class="space-y-space-sm font-journal-body text-[0.95rem] text-on-surface leading-relaxed">
                                    <p>"Late afternoon sunlight cuts cleanly across the white oak table. Captured a short study of shadows moving across the grain: a quiet analogue clock embedded right into the domestic geometry."</p>
                                    
                                    <div class="my-space-xs relative overflow-hidden rounded-lg bg-surface-container-highest shadow-sm aspect-[21/9] max-h-44">
                                        <img alt="Video thumbnail of light moving across oak desk" class="w-full h-full object-cover" src=(asset!("./img/oak.jpg"))>
                                        <div class="absolute inset-0 bg-black/20 flex items-center justify-center">
                                            <div class="w-9 h-9 rounded-full bg-surface/90 text-primary flex items-center justify-center shadow transition-transform hover:scale-105">
                                                <span class="material-symbols-outlined text-[18px] ml-0.5">"play_arrow"</span>
                                            </div>
                                        </div>
                                    </div>
                                </div>
                            </article>

                            
                            <div class="pt-space-md pb-space-xs flex flex-col items-center justify-center text-center opacity-60">
                                <span class="material-symbols-outlined text-[14px] text-on-surface-variant mb-0.5">"arrow_downward"</span>
                                <span class="font-ui-caption text-ui-caption text-on-surface-variant tracking-wider uppercase text-[9px]">"October 12, 2025 & earlier"</span>
                            </div>
                        </div>
                    </div>
                </div>
            </div>
        </section>
    })
}
