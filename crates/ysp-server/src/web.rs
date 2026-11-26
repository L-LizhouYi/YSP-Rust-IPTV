//! 内置网页：目录页 / 播放页 / 错误页（无模板引擎）。

use ysp_core::models::{Channel, Format, Program};

const CSS: &str = r#"
:root{color-scheme:dark}
*{box-sizing:border-box}
body{margin:0;background:#0f1115;color:#e7e9ee;font-family:-apple-system,BlinkMacSystemFont,"PingFang SC","Microsoft YaHei",sans-serif}
header{position:sticky;top:0;z-index:5;background:rgba(15,17,21,.92);backdrop-filter:blur(8px);border-bottom:1px solid #23262d}
.wrap{max-width:1180px;margin:0 auto;padding:0 18px}
header .wrap{display:flex;align-items:center;gap:16px;height:60px}
h1{font-size:17px;margin:0;font-weight:600;white-space:nowrap}
h1 span{color:#ff5c5c}
input[type=search]{flex:1;min-width:120px;background:#181b21;border:1px solid #2a2e37;color:#e7e9ee;border-radius:8px;padding:9px 12px;font-size:14px;outline:none}
input[type=search]:focus{border-color:#4a6cf7}
.meta{color:#8b93a3;font-size:12px;white-space:nowrap}
main{padding:22px 0 60px}
h2{font-size:14px;color:#9aa3b5;font-weight:600;margin:26px 0 12px;letter-spacing:.05em}
.grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(150px,1fr));gap:12px}
.card{display:block;background:#171a21;border:1px solid #23262d;border-radius:10px;padding:12px;text-decoration:none;color:inherit;transition:.15s}
.card:hover{border-color:#3d63ff;transform:translateY(-1px)}
.card .logo{height:44px;display:flex;align-items:center;margin-bottom:8px}
.card .logo img{max-height:40px;max-width:100%;object-fit:contain}
.card b{display:block;font-size:13px;font-weight:600;line-height:1.35;word-break:break-all}
.badge{display:inline-block;font-size:10px;padding:1px 6px;border-radius:99px;margin-top:6px}
.badge.vip{background:#3a1d24;color:#ff8a9b}
.badge.preview{background:#3a2f1a;color:#ffc766}
.badge.free{background:#1b2f22;color:#6fdc9b}
.badge.off{background:#262a33;color:#8b93a3}
.play{max-width:1000px;margin:0 auto;padding:18px}
video{width:100%;background:#000;border-radius:10px;aspect-ratio:16/9}
.bar{display:flex;gap:12px;align-items:center;flex-wrap:wrap;margin:14px 0}
.bar a{color:#9db4ff;text-decoration:none;font-size:13px}
.now{color:#c8cfdd;font-size:13px}
.tag{color:#8b93a3;font-size:12px}
details{margin-top:18px;background:#14171d;border:1px solid #23262d;border-radius:10px;padding:10px 14px}
summary{cursor:pointer;color:#9aa3b5;font-size:13px}
li{margin:4px 0;font-size:12px;color:#9aa3b5}
footer{border-top:1px solid #23262d;color:#5f6673;font-size:12px;padding:18px 0}
"#;

const SEARCH_JS: &str = r#"
const q=document.getElementById('q');
if(q){q.addEventListener('input',()=>{const v=q.value.trim().toLowerCase();
 document.querySelectorAll('.card').forEach(c=>{c.style.display=!v||c.dataset.k.includes(v)?'':'none'});
 document.querySelectorAll('section').forEach(s=>{const any=[...s.querySelectorAll('.card')].some(c=>c.style.display!=='none');s.style.display=any?'':'none'});});}
"#;

pub fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn badge(tier: &str, playable: Option<bool>) -> String {
    match (tier, playable) {
        ("vip", _) => "<span class=\"badge vip\">VIP</span>".into(),
        ("preview", _) => "<span class=\"badge preview\">限免</span>".into(),
        (_, Some(false)) => "<span class=\"badge off\">不可播</span>".into(),
        _ => "<span class=\"badge free\">免费</span>".into(),
    }
}

fn card(ch: &Channel, playable: Option<bool>, base: &str) -> String {
    let logo = if ch.logo.is_empty() {
        format!("<span class=\"tag\">{}</span>", escape(&ch.category))
    } else {
        format!(
            "<img src=\"{}\" loading=\"lazy\" alt=\"\">",
            escape(&ch.logo)
        )
    };
    let key = escape(&format!("{} {} {}", ch.name, ch.livepid, ch.category).to_lowercase());
    format!(
        "<a class=\"card\" href=\"{}/play/{}\" data-k=\"{}\">\
<div class=\"logo\">{}</div><b>{}</b>{}</a>",
        base,
        escape(ch.id()),
        key,
        logo,
        escape(&ch.name),
        badge(&ch.tier, playable)
    )
}

pub fn catalog_page(rows: &[(Channel, Option<bool>)], base: &str, stats: &str) -> String {
    let groups: [(&str, &str); 3] = [
        ("央视 / CGTN", "yangshi"),
        ("地方卫视", "weishi"),
        ("其它", ""),
    ];
    let mut sections = String::new();
    for (title, category) in groups {
        let items: Vec<&(Channel, Option<bool>)> = rows
            .iter()
            .filter(|(ch, _)| {
                if category.is_empty() {
                    ch.category != "yangshi" && ch.category != "weishi"
                } else {
                    ch.category == category
                }
            })
            .collect();
        if items.is_empty() {
            continue;
        }
        let cards: String = items
            .iter()
            .map(|(ch, playable)| card(ch, *playable, base))
            .collect();
        sections.push_str(&format!(
            "<section><h2>{} · {}</h2><div class=\"grid\">{}</div></section>",
            title,
            items.len(),
            cards
        ));
    }
    if sections.is_empty() {
        sections.push_str("<p>暂无频道</p>");
    }
    format!(
        "<!doctype html><html lang=\"zh-CN\"><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
<title>央视频点播</title><style>{}</style>\
<header><div class=\"wrap\"><h1>央视频<span>点播</span></h1>\
<input id=\"q\" type=\"search\" placeholder=\"搜索频道（CCTV13 / 湖南卫视 …）\">\
<div class=\"meta\">{}</div></div></header>\
<main class=\"wrap\">{}</main>\
<footer><div class=\"wrap\">仅用于个人技术研究与本地观看 · 内容版权归权利人所有</div></footer>\
<script>{}</script></html>",
        CSS,
        escape(stats),
        sections,
        SEARCH_JS
    )
}

const PLAYER_TMPL: &str = r#"<!doctype html><html lang="zh-CN"><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>%NAME% · 央视频点播</title><style>%CSS%</style>
<header><div class="wrap"><h1><a href="%BASE%/" style="color:inherit;text-decoration:none">← </a>央视频<span>点播</span></h1>
<div class="meta">%NAME%</div></div></header>
<main><div class="play">
<video id="v" controls autoplay muted playsinline></video>
<div class="bar"><b>%NAME%</b>%BADGE%
<span class="tag">%FMT%</span>
<a href="%BASE%/live/%ID%.m3u8">m3u8</a>
<a href="%BASE%/channels.m3u">频道列表</a>
<a href="%BASE%/epg.xml">节目单</a>
<a href="%BASE%/api/channels/%ID%/epg" target="_blank">当日节目</a></div>
%NOW%
<details><summary>播放器信息</summary><ul>
<li>id: %ID%</li><li>livepid: %LIVEPID%</li>
<li>cnlid: %CNLID%</li><li>清晰度: %DEFN%</li>
<li>线路: %MODE%</li></ul></details>
</div></main>
<footer><div class="wrap">仅用于个人技术研究与本地观看 · 内容版权归权利人所有</div></footer>
<script src="%BASE%/static/hls.min.js"></script>
<script>
(function(){
  var v=document.getElementById('v');
  var src='%BASE%/live/%ID%/master.m3u8';
  function kick(){try{v.play().catch(function(){});}catch(e){}}
  if(window.Hls&&Hls.isSupported()){
    var h=new Hls({lowLatencyMode:false});
    h.on(Hls.Events.MANIFEST_PARSED,function(){kick();});
    h.on(Hls.Events.FRAG_BUFFERED,function(){if(v.paused)kick();});
    h.loadSource(src);h.attachMedia(v);
  } else if(v.canPlayType('application/vnd.apple.mpegurl')){v.src=src;}
  v.addEventListener('canplay',kick);
  document.addEventListener('visibilitychange',function(){if(!document.hidden)kick();});
  kick();
})();
</script></html>"#;

pub fn player_page(
    ch: &Channel,
    base: &str,
    program: Option<&Program>,
    defn: &str,
    formats: &[Format],
    playable: Option<bool>,
    mode: &str,
) -> String {
    let fmt = formats
        .iter()
        .map(|f| {
            if f.label.is_empty() {
                f.fn_.clone()
            } else {
                f.label.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" · ");
    let now = match program {
        Some(p) => format!(
            "<div class=\"now\">正在播出：{} （{}–{}）</div>",
            escape(&p.name),
            escape(&p.start_time),
            escape(&p.end_time)
        ),
        None => String::new(),
    };
    PLAYER_TMPL
        .replace("%CSS%", CSS)
        .replace("%BASE%", base)
        .replace("%ID%", ch.id())
        .replace("%NAME%", &escape(&ch.name))
        .replace("%BADGE%", &badge(&ch.tier, playable))
        .replace("%FMT%", &escape(&fmt))
        .replace("%NOW%", &now)
        .replace("%LIVEPID%", &escape(&ch.livepid))
        .replace("%CNLID%", &escape(&ch.cnlid))
        .replace("%DEFN%", if defn.is_empty() { "-" } else { defn })
        .replace("%MODE%", mode)
}

pub fn error_page(code: u16, message: &str, base: &str) -> String {
    format!(
        "<!doctype html><html lang=\"zh-CN\"><meta charset=\"utf-8\">\
<title>{code}</title><style>{}</style>\
<main class=\"wrap\"><h2>{code}</h2><p>{}</p>\
<p><a href=\"{base}/\" style=\"color:#9db4ff\">返回目录</a></p></main></html>",
        CSS,
        escape(message)
    )
}
