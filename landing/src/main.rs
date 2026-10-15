mod landing;
mod waitlist;

use topcoat::asset::{AssetBundle, RouterBuilderAssetExt};

#[tokio::main]
async fn main() {
    topcoat::start(
        topcoat::router::Router::builder()
            .layout(landing::root_layout)
            .page(landing::home)
            .route(waitlist::join)
            .assets(AssetBundle::load().unwrap())
            .app_context(waitlist::WishlistStore::from_env())
            .build(),
    )
    .await
    .unwrap();
}
