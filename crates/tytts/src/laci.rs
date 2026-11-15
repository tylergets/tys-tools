use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

// Alignment heads from the same pinned release as the Zonos weights.
const BASE: &str = "https://huggingface.co/Zyphra/ZONOS3-Mini/resolve/d2d142a86b1e6aeb4252679673c86ecd86177bf8/web/laci";

pub fn language(value: &str) -> String {
    match value.to_lowercase().replace('-', "_").as_str() {
        "en" => "en_us".into(),
        "fr" => "fr_fr".into(),
        "pt" => "pt_br".into(),
        "zh" => "cmn".into(),
        other => other.into(),
    }
}

pub fn prepare(config: &crate::Config, dir: &Path, quiet: bool) -> Result<PathBuf> {
    let lang = language(config.language.as_deref().unwrap_or("en_us"));
    let Some(&(_, name, bytes, checksum)) = HEADS.iter().find(|row| row.0 == lang) else {
        bail!("unsupported Zonos language {lang:?}");
    };
    if config.model.is_none() {
        crate::download(
            dir,
            BASE,
            &crate::Asset {
                local: name,
                remote: name,
                bytes,
                checksum,
            },
            config.auto_download,
            quiet,
        )?;
    } else if !dir.join(name).is_file() {
        bail!(
            "custom Zonos bundle needs alignment heads: {}",
            dir.join(name).display()
        );
    }
    Ok(dir.join(name))
}

pub fn path(dir: &Path, lang: &str) -> Result<PathBuf> {
    let lang = language(lang);
    HEADS
        .iter()
        .find(|row| row.0 == lang)
        .map(|row| dir.join(row.1))
        .ok_or_else(|| anyhow::anyhow!("unsupported Zonos language {lang:?}"))
}

#[rustfmt::skip]
const HEADS: &[(&str, &str, u64, &str)] = &[
    ("en_us", "zonos3_mini_heads.json", 7495, "407e47472224c603b9c3a969e7175f35b637e0c1995eb606d5128ec1b2e88528"),
    ("en_gb", "zonos3_mini_heads.json", 7495, "407e47472224c603b9c3a969e7175f35b637e0c1995eb606d5128ec1b2e88528"),
    ("fr_fr", "zonos3_mini_heads_fr.json", 7571, "6ceacd45c4618504350532eb5c2628aec69ac8f18911256fa4378653216733d3"),
    ("de", "zonos3_mini_heads_de.json", 7539, "5029f11f7a75c02f68229f87f774e0440dc7bc98c091e8086828a1645d200645"),
    ("es", "zonos3_mini_heads_es.json", 7571, "20f0e8fae411ca651ddf6ae7fcab63e6b2d94340aa5497ca541b349feeb2d754"),
    ("it", "zonos3_mini_heads_it.json", 7557, "bd1ab97819057fd0b7c1cb6577a646118b42e1ba7568dff4ee379e925a74e2e7"),
    ("pt_br", "zonos3_mini_heads_pt.json", 7557, "cd675f1943ef3b06ee4d33a9c8fc55c0b3d0b41aedcf538889c89965b75036fb"),
    ("ja", "zonos3_mini_heads_ja.json", 7584, "1da810b373a7ddc591ffc654de363d8024821a407665083748c3eef5bf3e233a"),
    ("cmn", "zonos3_mini_heads_zh.json", 7558, "e8ef4f98d7738ca3a5768a125cad4a7362297580f9fcc46581c3eb025fd9273f"),
    ("ko", "zonos3_mini_heads_ko.json", 7566, "b5ef1551a418333ddc37a74078a825da718ac6434400a46efa98f5e5472c9fb9"),
    ("ar", "zonos3_mini_heads_ar.json", 7661, "e8d283a8eb0647015e26e2c6d49d2ec6fa2bd9941e5390e7b30396ca5e035ed6"),
    ("bn", "zonos3_mini_heads_bn.json", 7654, "9b49a48f9b7bfaf8857ee759736b786f3940c1890c394a4cdbe679e1f522e45e"),
    ("bg", "zonos3_mini_heads_bg.json", 7608, "1c4820a421ab318cd62ecc49e52ea4ec9aff520fe510ab54e31a654e394f6828"),
    ("cs", "zonos3_mini_heads_cs.json", 7571, "97ce6b22613053a17463deccbcd4943c1450c4f73210b09e93eec7679683eff1"),
    ("da", "zonos3_mini_heads_da.json", 7616, "bc1971f6054b465b939141bb06d0d07682501e74652fa0023d3f0a2db0c70146"),
    ("nl", "zonos3_mini_heads_nl.json", 7626, "5a28c72337284768512d274052e76e06a2b804447fcb0b7840740bbf77b631fe"),
    ("fi", "zonos3_mini_heads_fi.json", 7580, "7b7ece889fef904c39ca569d882746d61b105c37a348838041a72aa2164c43ef"),
    ("ka", "zonos3_mini_heads_ka.json", 7568, "854646c45094c81f5ff1cd7719a6eba1e13d45ed05676b3b4d2fe68ce14649cf"),
    ("el", "zonos3_mini_heads_el.json", 7603, "511ccc46c94476a01a0fe3fd96ee2b7e8926b1e5852772000faa9df54e92cee7"),
    ("gu", "zonos3_mini_heads_gu.json", 7362, "2282bc9873b50cd86e0fe84e0d4478df0558cbfbea9f5e0683f80de6fbe1383f"),
    ("he", "zonos3_mini_heads_he.json", 7639, "4cb8baae3d74b3099a68c07e9c69dc388b5c05755ee3544bd2da0860e1ba3956"),
    ("hi", "zonos3_mini_heads_hi.json", 7579, "0ef8179db4db8e29014faaebdbab9494d33a2974cf21405a884d6269eca1630a"),
    ("hu", "zonos3_mini_heads_hu.json", 7614, "8d42f5c087bb205a2e67509324ce999c5cdb56b26fd29685664bc8fe10dce143"),
    ("id", "zonos3_mini_heads_id.json", 7597, "0b2fe2fbc497b4f821d83488061ca32e6283ce2080975d4986e7c209a619dbad"),
    ("kn", "zonos3_mini_heads_kn.json", 7698, "0a0cf44eab3ebd28266e91750b32383d1e0ac5aa7a87a1e3d812145530b74307"),
    ("ms", "zonos3_mini_heads_ms.json", 7564, "723dd6e8c6a63f7eb157c12e9676d7b8ee44de2253a32887bb64483df874a70e"),
    ("ml", "zonos3_mini_heads_ml.json", 7639, "99d02ad7afeebe8f08b51ad2b6cde84c2a689c49c5644aab1693ff4937a1b66f"),
    ("mr", "zonos3_mini_heads_mr.json", 7591, "94d5414ef4c5a76a37b9bce02045e08b5f9a75bffca8379e9411301fcece8a27"),
    ("no", "zonos3_mini_heads_no.json", 7566, "2ce69edde434697ce97e10e4b7c65fbc90bd1d075533dbd1abd27e99d1774a6f"),
    ("or", "zonos3_mini_heads_or.json", 7434, "ce438e2cdcfa6e21ee9cddddeb03ea3fae519e8c81ed5a6b485ac03c356490b5"),
    ("pl", "zonos3_mini_heads_pl.json", 7569, "627b2d45c108d7be9afe0d603c24bdff69504145b750b1e3ef06065bad940e45"),
    ("pa", "zonos3_mini_heads_pa.json", 7582, "06ab88201f37b55b5f8ec87fcf9f28e002fc2ca3d983e5257bc2359d3aba531a"),
    ("ro", "zonos3_mini_heads_ro.json", 7574, "f42815de5929d821cbba40497e97d9cae284791b0651445d825f67e15f1205c9"),
    ("ru", "zonos3_mini_heads_ru.json", 7573, "63be82dca8a36445affdfd6483f31b5f4d3c0770a37b98cb791857a91d7fea3c"),
    ("sk", "zonos3_mini_heads_sk.json", 7578, "a9323f447a98d91af68d1f5d69f703f833f14f23467ddfe122ff052761127422"),
    ("sv", "zonos3_mini_heads_sv.json", 7584, "b710183a79178c6aa2ebe2699ec5947f6e1308d21107c166caf161f98c59af3b"),
    ("tl", "zonos3_mini_heads_tl.json", 7602, "6e76d81cff51edff3acf96b3bc52891e7c9aa02b1b217d5c8561287bafffb381"),
    ("ta", "zonos3_mini_heads_ta.json", 7452, "4b441fdbfa47587b560c1e3cef388aa0842191fc15c651b81cd65e3c88a6b8e0"),
    ("te", "zonos3_mini_heads_te.json", 7459, "c93da663899e460d08de7b158f765e963e231137369c2f7458240aaf3b4a2acf"),
    ("th", "zonos3_mini_heads_th.json", 7630, "b882630f4dfd28a1f610b99a9cdc3f8b22ce3abaa1b6b84d273149e32bbc0c92"),
    ("tr", "zonos3_mini_heads_tr.json", 7679, "f48baeea20cdb4bb84b84175a00a8a2307fcb4c5e0978bbb77aa7c856ed7a22e"),
    ("uk", "zonos3_mini_heads_uk.json", 7631, "f5f6c66f31281e287b21b45cf6b4ab722349e8299c48ed1c0396135edb44c6cc"),
    ("ur", "zonos3_mini_heads_ur.json", 7628, "bc2c557897ceafcfa8494f8321fa87c22b4b1ce5ae14b2407a91d843a3ffae32"),
    ("vi", "zonos3_mini_heads_vi.json", 7644, "0ed6768e0fa3aa1a5c444319dbbe813137d0a4083bc0c1dc53bedb06b9635030"),
];
