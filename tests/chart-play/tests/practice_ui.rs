//! Real slider, button and touch-consumption code with recording graphics/input adapters.
#![allow(dead_code)]
use std::{cell::{Cell,RefCell},collections::HashMap,ops::Range};
use phira_chart_play_tests::{practice_speed,practice_view};
thread_local! {
    static STATE:RefCell<HashMap<String,Option<u64>>>=RefCell::new(HashMap::new());
    static REQUEST:RefCell<Option<(String,String)>>=const{RefCell::new(None)};
    static REAL_TIME:Cell<f64>=const{Cell::new(0.)};
}
fn get_time()->f64{REAL_TIME.with(Cell::get)}
#[derive(Clone,Copy,Debug)]struct Vec2{x:f32,y:f32}
#[derive(Clone,Copy,Debug)]struct Rect{x:f32,y:f32,w:f32,h:f32}
impl Rect {
    fn new(x:f32,y:f32,w:f32,h:f32)->Self{Self{x,y,w,h}}
    fn feather(self,d:f32)->Self{Self::new(self.x-d,self.y-d,self.w+2.*d,self.h+2.*d)}
    fn right(self)->f32{self.x+self.w}
    fn bottom(self)->f32{self.y+self.h}
    fn rounded(self,_:f32)->Self{self}
    fn center(self)->Vec2{Vec2{x:self.x+self.w/2.,y:self.y+self.h/2.}}
    fn contains(self,p:Vec2)->bool{p.x>=self.x&&p.x<=self.x+self.w&&p.y>=self.y&&p.y<=self.y+self.h}
}
#[derive(Clone,Copy)]struct Color{a:f32}
const WHITE:Color=Color{a:1.};
#[derive(Clone,Copy,PartialEq)]enum TouchPhase{Started,Moved,Ended,Cancelled,Stationary}
struct Touch{id:u64,position:Vec2,phase:TouchPhase}
struct InputBox{text:String}
impl InputBox{fn new()->Self{Self{text:String::new()}}fn default_text(mut self,text:impl Into<String>)->Self{self.text=text.into();self}}
fn request_input(id:&str,b:InputBox){REQUEST.with(|r|*r.borrow_mut()=Some((id.into(),b.text)));}
#[derive(Default)]struct Ui{touches:Option<Vec<Touch>>,circles:Vec<(f32,f32,f32)>,texts:Vec<String>,paths:Vec<Rect>}
impl Ui {
    fn text(&mut self,s:impl Into<String>)->Text<'_>{let text=s.into();let count=text.chars().count();self.texts.push(text);Text{ui:self,count,size:0.4,x:0.,y:0.,width:10.}}
    fn ensure_touches(&mut self)->&mut Vec<Touch>{self.touches.get_or_insert_with(Vec::new)}
    fn fill_rect(&mut self,_:Rect,_:Color){}
    fn fill_path(&mut self,r:&Rect,_:Color){self.paths.push(*r);}
    fn fill_circle(&mut self,x:f32,y:f32,r:f32,_:Color){self.circles.push((x,y,r));}
    fn rect_to_global(&self,r:Rect)->Rect{r}
    fn to_local(&self,p:(f32,f32))->(f32,f32){p}
    fn accent(&self)->Color{WHITE}
    fn background(&self)->Color{WHITE}
}
struct Text<'a>{ui:&'a mut Ui,count:usize,size:f32,x:f32,y:f32,width:f32}
impl Text<'_>{
    fn size(mut self,s:f32)->Self{self.size=s;self}fn pos(mut self,x:f32,y:f32)->Self{self.x=x;self.y=y;self}
    fn anchor(self,_:f32,_:f32)->Self{self}fn max_width(mut self,w:f32)->Self{assert!(w>0.);self.width=w;self}
    fn color(self,_:Color)->Self{self}fn no_baseline(self)->Self{self}
    fn draw(self)->Rect{Rect::new(self.x,self.y,(self.count as f32*self.size*0.06).min(self.width),self.size*0.1)}
}
include!(concat!(env!("OUT_DIR"),"/practice_slider.rs"));
fn fixed_preview_frame(value:&mut f32,time:f64,phase:TouchPhase)->Ui {
    REAL_TIME.with(|clock|clock.set(time));
    let mut ui=Ui{touches:Some(vec![Touch{id:71,position:Vec2{x:0.58,y:0.115},phase}]),..Default::default()};
    ui.practice_speed_slider("fixed_preview_time","Preview",0.1..0.75,0.001,value,Some(0.4));
    ui
}
fn frame(value:&mut f32,id:&str,x:f32,y:f32,phase:TouchPhase)->Ui {
    let mut ui=Ui{touches:Some(vec![Touch{id:7,position:Vec2{x,y},phase}]),..Default::default()};
    ui.practice_speed_slider(id,"Speed",0.05..10.,0.05,value,Some(0.46));ui
}
#[test]
fn fixed_preview_short_press_changes_one_step_and_long_press_repeats_then_stops(){
    let mut value=0.45;
    fixed_preview_frame(&mut value,20.0,TouchPhase::Started);
    assert_eq!(value,0.45);
    fixed_preview_frame(&mut value,20.1,TouchPhase::Ended);
    assert!((value-0.451).abs()<1e-6);

    fixed_preview_frame(&mut value,30.0,TouchPhase::Started);
    fixed_preview_frame(&mut value,30.34,TouchPhase::Stationary);
    assert!((value-0.451).abs()<1e-6);
    fixed_preview_frame(&mut value,30.35,TouchPhase::Stationary);
    assert!((value-0.452).abs()<1e-6);
    fixed_preview_frame(&mut value,30.75,TouchPhase::Stationary);
    let early_steps=value-0.452;
    fixed_preview_frame(&mut value,31.15,TouchPhase::Stationary);
    let before_late_window=value;
    fixed_preview_frame(&mut value,31.55,TouchPhase::Stationary);
    assert!(value-before_late_window>early_steps);
    let released=value;
    fixed_preview_frame(&mut value,31.56,TouchPhase::Ended);
    assert_eq!(value,released);
    fixed_preview_frame(&mut value,32.0,TouchPhase::Moved);
    assert_eq!(value,released);
}

#[test]
fn other_practice_value_buttons_remain_single_step_after_a_long_hold(){
    let mut value=100.0;
    let plus=Rect::new(0.54,0.075,0.08,0.08).center();
    for (time,phase) in [(40.0,TouchPhase::Started),(41.0,TouchPhase::Stationary)] {
        REAL_TIME.with(|clock|clock.set(time));
        let mut ui=Ui{touches:Some(vec![Touch{id:72,position:plus,phase}]),..Default::default()};
        ui.practice_value_slider("exercise_view_scale","Scale",practice_view::SCALE,&mut value,Some(0.4));
    }
    assert_eq!(value,100.0);
    REAL_TIME.with(|clock|clock.set(41.1));
    let mut ui=Ui{touches:Some(vec![Touch{id:72,position:plus,phase:TouchPhase::Ended}]),..Default::default()};
    ui.practice_value_slider("exercise_view_scale","Scale",practice_view::SCALE,&mut value,Some(0.4));
    assert_eq!(value,101.0);
}
#[test]
fn dragging_visits_both_extremes_and_exact_midpoint(){
    let mut value=1.;
    let ui=frame(&mut value,"speed",0.23,0.115,TouchPhase::Started);
    assert_eq!(ui.circles[0].0,0.23);assert!(ui.texts.iter().any(|s|s=="1.00"));
    frame(&mut value,"speed",0.,0.115,TouchPhase::Moved);assert_eq!(value,0.05);
    frame(&mut value,"speed",0.23,0.115,TouchPhase::Moved);assert_eq!(value,1.);
    frame(&mut value,"speed",0.46,0.115,TouchPhase::Ended);assert_eq!(value,10.);
    STATE.with(|s|assert_eq!(s.borrow()["speed:drag"],None));
}
#[test]
fn numeric_button_follows_label_and_remains_clickable(){
    let mut value=1.;
    let mut ui=Ui::default();
    ui.practice_speed_slider("exercise_speed","Speed",0.05..10.,0.05,&mut value,Some(0.46));
    let number=ui.paths[0];assert!(number.right()<0.4);assert!(number.h>=0.07);
    let p=number.center();
    frame(&mut value,"exercise_speed",p.x,p.y,TouchPhase::Started);
    frame(&mut value,"exercise_speed",p.x,p.y,TouchPhase::Ended);
    REQUEST.with(|r|assert_eq!(r.borrow().as_ref().unwrap(),&("exercise_speed".into(),"1.00".into())));
    assert_eq!(value,1.);
    STATE.with(|s|assert_eq!(s.borrow()["exercise_speed:drag"],None));
}
#[test]
fn cancellation_does_not_commit_position_and_larger_buttons_remain_linear(){
    let mut value=1.;
    frame(&mut value,"speed",0.23,0.115,TouchPhase::Started);
    let ui=frame(&mut value,"speed",0.46,0.115,TouchPhase::Cancelled);assert_eq!(value,1.);
    let minus=ui.paths[1];let plus=ui.paths[2];
    assert_eq!(minus.w,0.08);assert_eq!(plus.w,0.08);assert!(plus.x-minus.right()>0.024);
    assert!(plus.right()+0.3<=1.); // actual practice panel starts at x=0.3
    for (button,expected) in [(plus,1.05),(minus,1.)] {
        let p=button.center();frame(&mut value,"speed",p.x,p.y,TouchPhase::Started);
        frame(&mut value,"speed",p.x,p.y,TouchPhase::Ended);assert!((value-expected).abs()<1e-6);
    }
}
#[test]
fn enlarged_hit_area_captures_and_empty_frames_do_not_release(){
    let mut value=1.;
    frame(&mut value,"speed",0.23,0.155,TouchPhase::Started); // 0.04 off the rail
    STATE.with(|s|assert_eq!(s.borrow()["speed:drag"],Some(7)));
    let mut ui=Ui::default();ui.practice_speed_slider("speed","Speed",0.05..10.,0.05,&mut value,Some(0.46));
    STATE.with(|s|assert_eq!(s.borrow()["speed:drag"],Some(7)));
    frame(&mut value,"speed",0.46,-0.8,TouchPhase::Moved);assert_eq!(value,10.);
    frame(&mut value,"speed",0.,0.8,TouchPhase::Ended);assert_eq!(value,0.05);
    STATE.with(|s|assert_eq!(s.borrow()["speed:drag"],None));
}
#[test]
fn owner_crossing_number_or_other_controls_never_clicks_them(){
    let mut value=1.;
    let ui=frame(&mut value,"speed",0.23,0.115,TouchPhase::Started);
    let p=ui.paths[0].center();
    frame(&mut value,"speed",p.x,p.y,TouchPhase::Moved);
    STATE.with(|s|assert_eq!(s.borrow()["speed:drag"],Some(7)));
    let mut ui=frame(&mut value,"speed",p.x,p.y,TouchPhase::Ended);
    REQUEST.with(|r|assert!(r.borrow().is_none()));
    assert!(ui.touches.as_ref().unwrap().is_empty());
    let mut other=1.;ui.practice_speed_slider("other","Flow",0.1..20.,0.05,&mut other,Some(0.46));assert_eq!(other,1.);
}
#[test]
fn unrelated_finger_release_does_not_end_drag_and_scene_cleanup_does(){
    let mut value=1.;frame(&mut value,"exercise_speed",0.23,0.115,TouchPhase::Started);
    let mut ui=Ui{touches:Some(vec![Touch{id:8,position:Vec2{x:0.,y:0.8},phase:TouchPhase::Ended}]),..Default::default()};
    ui.practice_speed_slider("exercise_speed","Speed",0.05..10.,0.05,&mut value,Some(0.46));
    STATE.with(|s|assert_eq!(s.borrow()["exercise_speed:drag"],Some(7)));
    Ui::clear_practice_speed_drag();
    STATE.with(|s|assert!(!s.borrow().contains_key("exercise_speed:drag")));
    frame(&mut value,"exercise_speed",0.46,0.115,TouchPhase::Moved);assert_eq!(value,1.);
}

#[test]
fn view_slider_uses_percentage_midpoint_and_preserves_outside_manual_value() {
    let mut value = 800.;
    let mut ui = Ui::default();
    ui.practice_value_slider("exercise_view_scale", "Size", practice_view::SCALE, &mut value, Some(0.4));
    assert_eq!(value, 800.);
    assert_eq!(ui.circles[0].0, 0.4);
    assert!(ui.texts.iter().any(|s| s == "800%"));
    ui.touches = Some(vec![Touch { id: 9, position: Vec2 { x: 0.2, y: 0.115 }, phase: TouchPhase::Started }]);
    ui.practice_value_slider("exercise_view_scale", "Size", practice_view::SCALE, &mut value, Some(0.4));
    assert_eq!(value, 100.);
    Ui::clear_practice_speed_drag();
    STATE.with(|s| assert!(!s.borrow().contains_key("exercise_view_scale:drag")));
}

#[test]
fn coordinate_slider_endpoints_and_center() {
    for (i, (p, expected)) in [(0., -2000.), (0.5, 0.), (1., 2000.)].into_iter().enumerate() {
        let mut value = 0.;
        let mut ui = Ui { touches: Some(vec![Touch { id: i as u64, position: Vec2 { x: 0.4 * p, y: 0.115 }, phase: TouchPhase::Started }]), ..Default::default() };
        ui.practice_value_slider("exercise_view_x", "X", practice_view::CENTER, &mut value, Some(0.4));
        assert_eq!(value, expected);
        Ui::clear_practice_speed_drag();
    }
}

#[test]
fn fixed_preview_slider_uses_the_whole_short_range_monotonically() {
    use phira_chart_play_tests::config::{FIXED_PREVIEW_TIME_MIN, FIXED_PREVIEW_TIME_MAX, FIXED_PREVIEW_TIME_STEP};
    let mut value = FIXED_PREVIEW_TIME_MIN;
    let mut previous = value;
    for i in 0..=20 {
        let position = i as f32 / 20.;
        let mut ui = Ui {
            touches: Some(vec![Touch { id: 11, position: Vec2 { x: 0.4 * position, y: 0.115 }, phase: TouchPhase::Started }]),
            ..Default::default()
        };
        ui.practice_speed_slider("fixed_preview_time", "Preview", FIXED_PREVIEW_TIME_MIN..FIXED_PREVIEW_TIME_MAX, FIXED_PREVIEW_TIME_STEP, &mut value, Some(0.4));
        let expected = FIXED_PREVIEW_TIME_MIN + (FIXED_PREVIEW_TIME_MAX - FIXED_PREVIEW_TIME_MIN) * position;
        assert!((value - expected).abs() <= FIXED_PREVIEW_TIME_STEP, "position={position}, value={value}, expected={expected}");
        assert!(value >= previous);
        // The rendered knob must match the touched coordinate, including both ends.
        assert!((ui.circles[0].0 - 0.4 * position).abs() < 0.001);
        previous = value;
        Ui::clear_practice_speed_drag();
    }
    assert_eq!(value, FIXED_PREVIEW_TIME_MAX);
}

#[test]
fn checked_box_has_an_explicit_check_mark() {
    let mut ui = Ui::default();
    let mut checked = true;
    ui.checkbox_with_id("fixed_preview_enabled", "Fixed Preview", &mut checked);
    assert!(ui.texts.iter().any(|text| text == "✓"));
}

#[test]
fn fixed_preview_panel_leaves_the_timeline_text_column_clear() {
    let (left, right) = practice_view::fixed_preview_panel_horizontal_bounds(true);
    assert!(left >= -1.0, "panel must remain on-screen: {left}");
    assert!(right <= -0.5, "panel intrudes into the centered timeline labels: {right}");
    let (top, bottom) = practice_view::fixed_preview_panel_vertical_bounds(true);
    for (screen_height, viewport_width) in [(1080., 1920.), (1080., 2400.)] {
        let aspect = viewport_width / screen_height;
        let timeline_bottom = practice_view::timeline_position(screen_height, viewport_width) + 0.06;
        assert!(top >= timeline_bottom + 0.02, "panel overlaps the timeline bar at {aspect}: {top} < {timeline_bottom}");
        assert!(bottom <= 1. / aspect - 0.005, "panel leaves the viewport at {aspect}: {bottom}");
    }
}
