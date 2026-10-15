

pub mod footer;
pub mod header;
pub mod hero;
pub mod manifesto;
pub mod roadmap;
pub mod showcase;

use topcoat::{
    Result,
    context::Cx,
    router::{Slot, layout, page, query_params},
    view::{View, view},
};

use topcoat::asset::asset;



const SITE_ORIGIN: &str = "https://dainiki.smbl.dev";

const OG_TITLE: &str = "Dainiki: a quiet, continuous river for your thoughts";
const OG_DESCRIPTION: &str = "A private, local-first daily journal crafted for those who value reflection over productivity metrics. No cards. No document switching. Just your life in an unbroken, beautifully typeset stream.";

const OG_IMAGE: &str = "https://dainiki.smbl.dev/og.png";


#[layout("/")]
pub async fn root_layout(slot: Slot<'_>) -> Result<impl View> {
    Ok(view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8">
                <meta content="width=device-width, initial-scale=1.0" name="viewport">
                <meta content="web_standard" name="shell-type">
                <title>(OG_TITLE)</title>
                <link rel="icon" type="image/png" href=(asset!("./landing/img/logo.png"))>
                <link rel="apple-touch-icon" href=(asset!("./landing/img/logo.png"))>
                
                <meta content="website" property="og:type">
                <meta content=(SITE_ORIGIN) property="og:url">
                <meta content="Dainiki" property="og:site_name">
                <meta content=(OG_TITLE) property="og:title">
                <meta content=(OG_DESCRIPTION) property="og:description">
                <meta content=(OG_IMAGE) property="og:image">
                <meta content="1200" property="og:image:width">
                <meta content="630" property="og:image:height">
                <meta content=(OG_TITLE) property="og:image:alt">
                
                <meta content="summary_large_image" name="twitter:card">
                <meta content=(OG_TITLE) name="twitter:title">
                <meta content=(OG_DESCRIPTION) name="twitter:description">
                <meta content=(OG_IMAGE) name="twitter:image">
                <link href="https://fonts.googleapis.com" rel="preconnect">
                <link crossorigin="" href="https://fonts.gstatic.com" rel="preconnect">
                <link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600&family=Newsreader:ital,opsz,wght@0,6..72,400;0,6..72,500;1,6..72,400&display=swap" rel="stylesheet">
                <link href="https://fonts.googleapis.com/css2?family=Material+Symbols+Outlined:opsz,wght,FILL,GRAD@20..48,100..700,0..1,-50..200" rel="stylesheet">
                <link rel="stylesheet" href=(topcoat::tailwind::stylesheet!())>
                topcoat::dev::script()
            </head>
            <body class="bg-surface font-ui-body text-ui-body text-on-surface">
                (slot)
            </body>
        </html>
    })
}


#[topcoat::router::query_params(error = bad_request)]
pub struct HomeQuery {
    pub joined: Option<String>,
}


#[derive(Clone, Copy)]
pub enum WaitlistStatus {
    Joined,
    Duplicate,
    Invalid,
    Error,
}

impl WaitlistStatus {
    fn from_code(code: &str) -> Option<Self> {
        match code {
            "joined" => Some(Self::Joined),
            "duplicate" => Some(Self::Duplicate),
            "invalid" => Some(Self::Invalid),
            "error" => Some(Self::Error),
            _ => None,
        }
    }

    fn message(self) -> &'static str {
        match self {
            Self::Joined => "You're on the list. We'll be in touch soon.",
            Self::Duplicate => "You're already on the waitlist.",
            Self::Invalid => "Please enter a valid email address.",
            Self::Error => "Something went wrong. Please try again in a moment.",
        }
    }

    fn tone_class(self) -> &'static str {
        match self {
            Self::Joined => "text-primary",
            Self::Duplicate => "text-on-surface-variant",
            Self::Invalid | Self::Error => "text-error",
        }
    }
}

#[page("/")]
pub async fn home(cx: &Cx) -> Result<impl View> {
    let status = query_params::<HomeQuery>(cx)?
        .joined
        .as_deref()
        .and_then(WaitlistStatus::from_code);

    Ok(view! {
        <div class="flex flex-col w-full">
            header::header()
            <main class="w-full pt-16 bg-surface">
                hero::hero(status: status)
                showcase::showcase()
                manifesto::pull_quote()
                roadmap::roadmap()
            </main>
            footer::footer()
        </div>
    })
}
