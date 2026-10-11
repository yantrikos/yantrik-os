use super::*;
use slint::platform::{Key,WindowEvent};
use slint::{ModelRc,VecModel};
fn chord(w:&MinimalSoftwareWindow, text:&str){
    w.dispatch_event(WindowEvent::KeyPressed{text:Key::Control.into()});key(w,text.into());w.dispatch_event(WindowEvent::KeyReleased{text:Key::Control.into()});
}
pub fn run(w:&MinimalSoftwareWindow, output:&str,width:u32,height:u32)->Result<(),Box<dyn std::error::Error>>{
    let ui=SettingsProbe::new()?;ui.set_canvas_width(width as f32);ui.set_canvas_height(height as f32);
    let all:Vec<_>=["Appearance","AI & Intelligence","Desktop","Network","Accounts","Privacy & Security","System","Skills","Harnesses"].iter().enumerate().map(|(i,s)|SettingsCategoryItem{id:i as i32,label:(*s).into(),icon:"".into()}).collect();
    ui.set_categories(ModelRc::new(VecModel::from(all.clone())));
    let weak=ui.as_weak();ui.on_search(move|q|{if let Some(ui)=weak.upgrade(){ui.set_categories(ModelRc::new(VecModel::from(all.iter().filter(|c|q.is_empty()||c.label.to_lowercase().contains(&q.to_lowercase())).cloned().collect::<Vec<_>>())));}});
    ui.show()?;w.set_size(slint::PhysicalSize::new(width,height));
    let draw=||{slint::platform::update_timers_and_animations();let mut p=slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width,height);w.request_redraw();w.draw_if_needed(|r|{r.render(p.make_mut_slice(),width as usize);});p};
    draw();
    // Enter a real category by keyboard, then clear with Escape.
    chord(w,"f");key(w,"Network".into());key(w,"\n".into());draw();assert_eq!(ui.get_category(),3);
    key(w,Key::Escape.into());draw();assert_eq!(ui.get_query(),"");
    click(w,80.,180.);draw();chord(w,"f");key(w,"Desktop".into());key(w,"\n".into());draw();assert_eq!(ui.get_category(),2,"Search works after focusing a category");
    key(w,Key::Escape.into());draw();assert_eq!(ui.get_query(),"");
    ui.set_dark(width>=1000);
    for category in 0..9{ui.set_category(category);let pixels=draw();
        if [2,3,4,5,6].contains(&category){
            let path=output.replace(".png",&format!("-category-{category}.png"));
            let f=BufWriter::new(File::create(path)?);let mut enc=png::Encoder::new(f,width,height);enc.set_color(png::ColorType::Rgb);enc.set_depth(png::BitDepth::Eight);enc.write_header()?.write_image_data(pixels.as_bytes())?;
        }
    }
    ui.set_category(5);draw();click(w,width as f32-75.,if width>=1000{536.}else{528.});assert_eq!(ui.get_action(),"screen:28","Permissions opens its own dashboard");
    ui.set_category(0);ui.set_dark(width>=1000);draw();
    // The theme cards (176px and the gap, 196 in all) sit above the accent row now: on a tall
    // canvas it is that much lower, on a short one it is scrolled up to where it was.
    let accent_y=if width>=1000{395.}else{413.};
    let accent_y=if height>=700{accent_y+196.}else{
        w.dispatch_event(WindowEvent::PointerScrolled{position:slint::LogicalPosition::new(width as f32-100.,height as f32-150.),delta_x:0.,delta_y:-196.});
        std::thread::sleep(std::time::Duration::from_millis(350));draw();accent_y};
    // The swatches as settings.slint lays them out: 38px each, no gap, flush right inside the
    // row's padding and the page's margin (44px in all), in this order. Clicked by place in this
    // list, not by a pixel number: two presets were taken out (#626) and a fixed x went on
    // clicking where Purple used to be, which is now the soft blue. Purple is clicked last, so
    // the keyboard check below presses the swatch that has focus.
    const SWATCHES:[&str;3]=["cyan","purple","pink"];
    let swatch_x=|id:&str|{let i=SWATCHES.iter().position(|s|*s==id).unwrap();width as f32-44.-(SWATCHES.len()-i) as f32*38.+19.};
    for id in ["cyan","pink","purple"]{click(w,swatch_x(id),accent_y);assert_eq!(ui.get_accent(),id,"Direct accent choice: the {id} swatch");}
    key(w," ".into());assert_eq!(ui.get_action(),"accent:purple","Accent keyboard activation");
    ui.set_accent("cyan".into());draw();
    let p=draw();let f=BufWriter::new(File::create(output)?);let mut e=png::Encoder::new(f,width,height);e.set_color(png::ColorType::Rgb);e.set_depth(png::BitDepth::Eight);e.write_header()?.write_image_data(p.as_bytes())?;
    w.dispatch_event(WindowEvent::PointerScrolled{position:slint::LogicalPosition::new(width as f32-100.,height as f32-150.),delta_x:0.,delta_y:-350.});
    std::thread::sleep(std::time::Duration::from_millis(350));let scrolled=draw();
    let f=BufWriter::new(File::create(output.replace(".png","-scrolled.png"))?);let mut e=png::Encoder::new(f,width,height);e.set_color(png::ColorType::Rgb);e.set_depth(png::BitDepth::Eight);e.write_header()?.write_image_data(scrolled.as_bytes())?;
    assert_ne!(p.as_bytes(),scrolled.as_bytes(),"Wheel input must move overflowing settings content");
    ui.set_save_error(true);draw();
    // Footer is always reachable, including on compact displays.
    click(w,width as f32-155.,height as f32-19.);assert_eq!(ui.get_action(),"retry");
    println!("PASS: Settings keyboard search, Enter, Escape, category focus, all nine pages, and visible save retry at {width}x{height}");Ok(())
}
