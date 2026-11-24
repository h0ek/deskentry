use crate::comparison::{self, KeyDifference};
use crate::model::{
    ActionLocalizedValue, DesktopAction, DesktopDocument, ExtraKeyValue, LocalizedValue,
};
use crate::scanner::{self, EntryGroup, SourceKind};
use crate::{storage, updates};
use adw::prelude::*;
use gtk::gio;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

const LOCALIZED_KEYS: [&str; 5] = ["Name", "GenericName", "Comment", "Keywords", "Icon"];
const BOOLEAN_STATES: [&str; 3] = ["Default", "True", "False"];
const ACTION_LOCALIZED_KEYS: [&str; 2] = ["Name", "Icon"];
const SOURCE_FILTERS: [&str; 6] = [
    "All sources",
    "User",
    "System",
    "Flatpak",
    "Snap",
    "Custom XDG",
];
const TYPE_FILTERS: [&str; 5] = ["All types", "Application", "Link", "Directory", "Other"];

#[derive(Clone, Debug, Eq, PartialEq)]
struct EditorSnapshot {
    id: String,
    name: String,
    generic_name: String,
    comment: String,
    exec: String,
    icon: String,
    categories: String,
    keywords: String,
    mime_type: String,
    entry_type: String,
    version: String,
    try_exec: String,
    path: String,
    url: String,
    only_show_in: String,
    not_show_in: String,
    implements: String,
    boolean_states: [u32; 7],
    localized_values: Vec<LocalizedValue>,
    actions: Vec<DesktopAction>,
    extra_values: Vec<ExtraKeyValue>,
}

#[derive(Clone)]
struct DirtySink {
    state: Weak<RefCell<AppState>>,
    indicator: gtk::Label,
    status: gtk::Label,
    save: gtk::Button,
}

#[derive(Clone, Debug)]
enum PendingAction {
    Select(String),
    Reload,
    New,
    Close,
    OpenFile,
}

#[derive(Clone)]
struct ExtraKeyRow {
    row: gtk::ListBoxRow,
    group: gtk::Entry,
    key: gtk::Entry,
    value: gtk::Entry,
}

struct HeaderFields {
    root: adw::HeaderBar,
    add: gtk::Button,
    open_file: gtk::Button,
    about: gtk::Button,
    reload: gtk::Button,
}

struct SidebarFields {
    root: gtk::Box,
    stack: gtk::Stack,
    search: gtk::SearchEntry,
    source_filter: gtk::DropDown,
    type_filter: gtk::DropDown,
    list: gtk::ListBox,
}

#[derive(Clone)]
struct LocaleRow {
    row: gtk::ListBoxRow,
    key: gtk::DropDown,
    locale: gtk::Entry,
    value: gtk::Entry,
}

#[derive(Clone)]
struct ActionLocaleRow {
    row: gtk::ListBoxRow,
    key: gtk::DropDown,
    locale: gtk::Entry,
    value: gtk::Entry,
}

#[derive(Clone)]
struct DesktopActionRow {
    row: gtk::ListBoxRow,
    original_id: Option<String>,
    id: gtk::Entry,
    name: gtk::Entry,
    exec: gtk::Entry,
    icon: gtk::Entry,
    localized_rows: Rc<RefCell<Vec<ActionLocaleRow>>>,
}

#[derive(Clone)]
struct EditorWidgets {
    id: gtk::Entry,
    name: gtk::Entry,
    generic_name: gtk::Entry,
    comment: gtk::Entry,
    exec: gtk::Entry,
    icon: gtk::Entry,
    icon_preview: gtk::Image,
    import_icon: gtk::Button,
    browse_icon: gtk::Button,
    categories: gtk::Entry,
    keywords: gtk::Entry,
    mime_type: gtk::Entry,
    entry_type: gtk::Entry,
    version: gtk::Entry,
    try_exec: gtk::Entry,
    path: gtk::Entry,
    url: gtk::Entry,
    only_show_in: gtk::Entry,
    not_show_in: gtk::Entry,
    implements: gtk::Entry,
    terminal: gtk::DropDown,
    no_display: gtk::DropDown,
    hidden: gtk::DropDown,
    startup_notify: gtk::DropDown,
    dbus_activatable: gtk::DropDown,
    prefers_non_default_gpu: gtk::DropDown,
    single_main_window: gtk::DropDown,
    source: gtk::Label,
    file_path: gtk::Label,
    override_info: gtk::Label,
    original_path: gtk::Label,
    view_original: gtk::Button,
    compare_status: gtk::Label,
    compare_list: gtk::ListBox,
    locale_list: gtk::ListBox,
    locale_rows: Rc<RefCell<Vec<LocaleRow>>>,
    add_locale: gtk::Button,
    action_list: gtk::ListBox,
    action_rows: Rc<RefCell<Vec<DesktopActionRow>>>,
    add_action: gtk::Button,
    raw_buffer: gtk::TextBuffer,
    extra_list: gtk::ListBox,
    extra_rows: Rc<RefCell<Vec<ExtraKeyRow>>>,
    add_extra: gtk::Button,
    page_stack: gtk::Stack,
    status: gtk::Label,
    dirty_indicator: gtk::Label,
    view_stack: gtk::Stack,
    dirty_sink: Rc<RefCell<Option<DirtySink>>>,
    reset_override: gtk::Button,
    delete_entry: gtk::Button,
    open_location: gtk::Button,
    launch: gtk::Button,
    save: gtk::Button,
    validate: gtk::Button,
    revert: gtk::Button,
}

#[derive(Clone)]
struct AppWidgets {
    window: adw::ApplicationWindow,
    list: gtk::ListBox,
    list_stack: gtk::Stack,
    search: gtk::SearchEntry,
    source_filter: gtk::DropDown,
    type_filter: gtk::DropDown,
    add: gtk::Button,
    reload: gtk::Button,
    open_file: gtk::Button,
    about: gtk::Button,
    editor: EditorWidgets,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum EntryPersistence {
    #[default]
    Existing,
    Unsaved,
}

impl EntryPersistence {
    fn needs_initial_save(self) -> bool {
        self == Self::Unsaved
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum CloseGuard {
    #[default]
    Guarded,
    Allow,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum SelectionSignal {
    #[default]
    Normal,
    SuppressNext,
}

#[derive(Default)]
struct AppState {
    groups: Vec<EntryGroup>,
    visible_ids: Vec<String>,
    selected_id: Option<String>,
    document: Option<DesktopDocument>,
    effective_path: Option<PathBuf>,
    baseline: Option<EditorSnapshot>,
    persistence: EntryPersistence,
    close_guard: CloseGuard,
    selection_signal: SelectionSignal,
}

struct GeneralFields {
    id: gtk::Entry,
    name: gtk::Entry,
    generic_name: gtk::Entry,
    comment: gtk::Entry,
    exec: gtk::Entry,
    icon: gtk::Entry,
    icon_preview: gtk::Image,
    import_icon: gtk::Button,
    browse_icon: gtk::Button,
    categories: gtk::Entry,
    keywords: gtk::Entry,
    mime_type: gtk::Entry,
}

struct EditorParts {
    general: GeneralFields,
    advanced: AdvancedFields,
    source: SourceFields,
    locale: LocalePageFields,
    action: ActionPageFields,
    raw: RawPageFields,
    actions: ActionFields,
    page_stack: gtk::Stack,
    view_stack: gtk::Stack,
}

struct AdvancedFields {
    entry_type: gtk::Entry,
    version: gtk::Entry,
    try_exec: gtk::Entry,
    path: gtk::Entry,
    url: gtk::Entry,
    only_show_in: gtk::Entry,
    not_show_in: gtk::Entry,
    implements: gtk::Entry,
    terminal: gtk::DropDown,
    no_display: gtk::DropDown,
    hidden: gtk::DropDown,
    startup_notify: gtk::DropDown,
    dbus_activatable: gtk::DropDown,
    prefers_non_default_gpu: gtk::DropDown,
    single_main_window: gtk::DropDown,
}

struct SourceFields {
    source: gtk::Label,
    file_path: gtk::Label,
    override_info: gtk::Label,
    original_path: gtk::Label,
    view_original: gtk::Button,
    compare_status: gtk::Label,
    compare_list: gtk::ListBox,
}

struct RawPageFields {
    buffer: gtk::TextBuffer,
    extra_list: gtk::ListBox,
    add_extra: gtk::Button,
}

struct LocalePageFields {
    list: gtk::ListBox,
    add: gtk::Button,
}

struct ActionPageFields {
    list: gtk::ListBox,
    add: gtk::Button,
}

struct ActionFieldWidgets {
    grid: gtk::Grid,
    id: gtk::Entry,
    name: gtk::Entry,
    exec: gtk::Entry,
    icon: gtk::Entry,
    remove: gtk::Button,
}

struct ActionFields {
    status: gtk::Label,
    dirty_indicator: gtk::Label,
    reset_override: gtk::Button,
    delete_entry: gtk::Button,
    open_location: gtk::Button,
    launch: gtk::Button,
    save: gtk::Button,
    validate: gtk::Button,
    revert: gtk::Button,
}

struct SelectedEntry {
    id: String,
    effective: scanner::EntryVariant,
    variant_count: usize,
    lower_path: Option<String>,
}

struct VisibleEntry {
    id: String,
    name: String,
    icon: String,
    entry_type: String,
    source: SourceKind,
}

pub fn build_ui(app: &adw::Application) {
    let state = Rc::new(RefCell::new(AppState::default()));
    let widgets = build_window(app);

    install_dirty_sink(&state, &widgets.editor);
    connect_search(&state, &widgets);
    connect_filters(&state, &widgets);
    connect_selection(&state, &widgets);
    connect_toolbar(&state, &widgets);
    connect_about(&widgets);
    connect_editor_actions(&state, &widgets);
    connect_icon_import(&widgets);
    connect_icon_browser(&widgets);
    connect_icon_preview(&widgets.editor);
    connect_dirty_tracking(&widgets.editor);
    connect_locale_add(&widgets.editor);
    connect_action_add(&widgets.editor);
    connect_extra_add(&widgets.editor);
    connect_type_awareness(&widgets.editor);
    connect_page_refresh(&state, &widgets);
    connect_close_guard(&state, &widgets);
    connect_window_actions(app, &state, &widgets);

    reload_entries(&state, &widgets, None);
    widgets.window.present();
    check_startup_update(&widgets.window);
}

fn build_window(app: &adw::Application) -> AppWidgets {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("DeskEntry")
        .default_width(1180)
        .default_height(760)
        .build();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let header = build_header();
    root.append(&header.root);

    let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
    paned.set_position(330);
    paned.set_wide_handle(true);
    paned.set_hexpand(true);
    paned.set_vexpand(true);

    let sidebar = build_sidebar();
    let (editor_root, editor) = build_editor();
    paned.set_start_child(Some(&sidebar.root));
    paned.set_end_child(Some(&editor_root));
    paned.set_resize_start_child(false);
    paned.set_shrink_start_child(false);
    root.append(&paned);
    window.set_content(Some(&root));

    AppWidgets {
        window,
        list: sidebar.list,
        list_stack: sidebar.stack,
        search: sidebar.search,
        source_filter: sidebar.source_filter,
        type_filter: sidebar.type_filter,
        add: header.add,
        reload: header.reload,
        open_file: header.open_file,
        about: header.about,
        editor,
    }
}

fn build_header() -> HeaderFields {
    let root = adw::HeaderBar::new();
    let title = gtk::Label::new(Some("DeskEntry"));
    title.add_css_class("title");
    root.set_title_widget(Some(&title));

    let add = gtk::Button::from_icon_name("list-add-symbolic");
    add.set_tooltip_text(Some("New desktop entry"));
    let open_file = gtk::Button::from_icon_name("document-open-symbolic");
    open_file.set_tooltip_text(Some("Open desktop file"));
    let about = gtk::Button::from_icon_name("help-about-symbolic");
    about.set_tooltip_text(Some("About DeskEntry"));
    let reload = gtk::Button::from_icon_name("view-refresh-symbolic");
    reload.set_tooltip_text(Some("Reload desktop entries"));

    root.pack_start(&add);
    root.pack_start(&open_file);
    root.pack_start(&about);
    root.pack_end(&reload);

    HeaderFields {
        root,
        add,
        open_file,
        about,
        reload,
    }
}

fn build_sidebar() -> SidebarFields {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    root.set_margin_top(12);
    root.set_margin_bottom(12);
    root.set_margin_start(12);
    root.set_margin_end(6);
    root.set_size_request(280, -1);

    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("Search applications"));
    root.append(&search);

    let filters = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let source_filter = gtk::DropDown::from_strings(&SOURCE_FILTERS);
    source_filter.set_hexpand(true);
    source_filter.set_tooltip_text(Some("Filter by desktop entry source"));
    let type_filter = gtk::DropDown::from_strings(&TYPE_FILTERS);
    type_filter.set_hexpand(true);
    type_filter.set_tooltip_text(Some("Filter by desktop entry type"));
    filters.append(&source_filter);
    filters.append(&type_filter);
    root.append(&filters);

    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Single);
    list.add_css_class("boxed-list");
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&list)
        .build();

    let empty = adw::StatusPage::builder()
        .icon_name("system-search-symbolic")
        .title("No matching entries")
        .description("Adjust the search text or filters.")
        .build();
    empty.add_css_class("compact");

    let stack = gtk::Stack::new();
    stack.set_vexpand(true);
    stack.add_named(&scroll, Some("list"));
    stack.add_named(&empty, Some("empty"));
    stack.set_visible_child_name("list");
    root.append(&stack);

    SidebarFields {
        root,
        stack,
        search,
        source_filter,
        type_filter,
        list,
    }
}

fn build_editor() -> (gtk::Box, EditorWidgets) {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    outer.set_hexpand(true);
    outer.set_vexpand(true);

    let empty = adw::StatusPage::builder()
        .icon_name("application-x-executable-symbolic")
        .title("Select a desktop entry")
        .description("Choose an entry from the list or create a new one.")
        .build();

    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    root.set_margin_top(12);
    root.set_margin_bottom(12);
    root.set_margin_start(6);
    root.set_margin_end(12);
    root.set_hexpand(true);
    root.set_vexpand(true);

    let stack = gtk::Stack::new();
    stack.set_hexpand(true);
    stack.set_vexpand(true);
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    let switcher = gtk::StackSwitcher::new();
    switcher.set_stack(Some(&stack));
    switcher.set_halign(gtk::Align::Center);
    root.append(&switcher);

    let (general_page, general) = build_general_page();
    let (advanced_page, advanced) = build_advanced_page();
    let (locale_page, locale_fields) = build_locale_page();
    let (actions_page, action_fields) = build_actions_page();
    let (source_page, source_fields) = build_source_page();
    let (raw_page, raw_fields) = build_raw_page();
    stack.add_titled(&general_page, Some("general"), "General");
    stack.add_titled(&advanced_page, Some("advanced"), "Advanced");
    stack.add_titled(&locale_page, Some("locales"), "Locales");
    stack.add_titled(&actions_page, Some("actions"), "Actions");
    stack.add_titled(&source_page, Some("source"), "Source");
    stack.add_titled(&raw_page, Some("raw"), "Raw / Extra");
    root.append(&stack);

    let actions = build_action_bar();
    root.append(&build_status_row(&actions));
    root.append(&build_action_buttons(&actions));

    let view_stack = gtk::Stack::new();
    view_stack.set_hexpand(true);
    view_stack.set_vexpand(true);
    view_stack.add_named(&empty, Some("empty"));
    view_stack.add_named(&root, Some("editor"));
    view_stack.set_visible_child_name("empty");
    outer.append(&view_stack);

    let widgets = combine_editor_widgets(EditorParts {
        general,
        advanced,
        source: source_fields,
        locale: locale_fields,
        action: action_fields,
        raw: raw_fields,
        actions,
        page_stack: stack,
        view_stack,
    });
    set_editor_sensitive(&widgets, false);
    (outer, widgets)
}

fn build_general_page() -> (gtk::ScrolledWindow, GeneralFields) {
    let grid = form_grid();
    let id = add_entry_row(&grid, 0, "Desktop ID");
    let name = add_entry_row(&grid, 1, "Name");
    let generic_name = add_entry_row(&grid, 2, "Generic name");
    let comment = add_entry_row(&grid, 3, "Comment");
    let exec = add_entry_row(&grid, 4, "Exec");

    let icon_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let icon_preview = gtk::Image::from_icon_name("application-x-executable");
    icon_preview.set_pixel_size(32);
    let icon = gtk::Entry::new();
    icon.set_hexpand(true);
    let browse_icon = gtk::Button::with_label("Browse");
    let import_icon = gtk::Button::with_label("Import");
    icon_box.append(&icon_preview);
    icon_box.append(&icon);
    icon_box.append(&browse_icon);
    icon_box.append(&import_icon);
    grid.attach(&form_label("Icon"), 0, 5, 1, 1);
    grid.attach(&icon_box, 1, 5, 1, 1);

    let categories = add_entry_row(&grid, 6, "Categories");
    let keywords = add_entry_row(&grid, 7, "Keywords");
    let mime_type = add_entry_row(&grid, 8, "MIME types");
    let fields = GeneralFields {
        id,
        name,
        generic_name,
        comment,
        exec,
        icon,
        icon_preview,
        import_icon,
        browse_icon,
        categories,
        keywords,
        mime_type,
    };
    (scrolled(&grid), fields)
}

fn build_advanced_page() -> (gtk::ScrolledWindow, AdvancedFields) {
    let grid = form_grid();
    let entry_type = add_entry_row(&grid, 0, "Type");
    entry_type.set_placeholder_text(Some("Application, Link or Directory"));
    let version = add_entry_row(&grid, 1, "Spec version");
    version.set_placeholder_text(Some("1.5"));
    version.set_tooltip_text(Some(
        "Desktop Entry Specification version, not the application version",
    ));
    let try_exec = add_entry_row(&grid, 2, "TryExec");
    let path = add_entry_row(&grid, 3, "Path");
    let url = add_entry_row(&grid, 4, "URL");
    let only_show_in = add_entry_row(&grid, 5, "OnlyShowIn");
    let not_show_in = add_entry_row(&grid, 6, "NotShowIn");
    let implements = add_entry_row(&grid, 7, "Implements");
    let terminal = add_boolean_row(&grid, 8, "Terminal");
    let no_display = add_boolean_row(&grid, 9, "NoDisplay");
    let hidden = add_boolean_row(&grid, 10, "Hidden");
    let startup_notify = add_boolean_row(&grid, 11, "StartupNotify");
    let dbus_activatable = add_boolean_row(&grid, 12, "DBusActivatable");
    let prefers_non_default_gpu = add_boolean_row(&grid, 13, "PrefersNonDefaultGPU");
    let single_main_window = add_boolean_row(&grid, 14, "SingleMainWindow");
    let fields = AdvancedFields {
        entry_type,
        version,
        try_exec,
        path,
        url,
        only_show_in,
        not_show_in,
        implements,
        terminal,
        no_display,
        hidden,
        startup_notify,
        dbus_activatable,
        prefers_non_default_gpu,
        single_main_window,
    };
    (scrolled(&grid), fields)
}

fn build_locale_page() -> (gtk::Box, LocalePageFields) {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 8);
    page.set_margin_top(12);
    page.set_margin_bottom(12);
    page.set_margin_start(12);
    page.set_margin_end(12);
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    let scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&list)
        .build();
    let add = gtk::Button::with_label("Add translation");
    add.set_halign(gtk::Align::Start);
    page.append(&scroll);
    page.append(&add);
    (page, LocalePageFields { list, add })
}

fn build_actions_page() -> (gtk::Box, ActionPageFields) {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 8);
    page.set_margin_top(12);
    page.set_margin_bottom(12);
    page.set_margin_start(12);
    page.set_margin_end(12);

    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    list.add_css_class("boxed-list");
    let scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&list)
        .build();
    let add = gtk::Button::with_label("Add action");
    add.set_halign(gtk::Align::Start);
    page.append(&scroll);
    page.append(&add);
    (page, ActionPageFields { list, add })
}

fn build_source_page() -> (gtk::Box, SourceFields) {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 12);
    page.set_margin_top(12);
    page.set_margin_bottom(12);
    page.set_margin_start(12);
    page.set_margin_end(12);

    let grid = form_grid();
    let source = source_label();
    let file_path = source_label();
    file_path.set_selectable(true);
    let override_info = source_label();
    let original_path = source_label();
    original_path.set_selectable(true);
    grid.attach(&form_label("Source"), 0, 0, 1, 1);
    grid.attach(&source, 1, 0, 1, 1);
    grid.attach(&form_label("File"), 0, 1, 1, 1);
    grid.attach(&file_path, 1, 1, 1, 1);
    grid.attach(&form_label("Override"), 0, 2, 1, 1);
    grid.attach(&override_info, 1, 2, 1, 1);
    grid.attach(&form_label("Original"), 0, 3, 1, 1);
    grid.attach(&original_path, 1, 3, 1, 1);
    page.append(&grid);

    let view_original = gtk::Button::with_label("View Original Source");
    view_original.set_halign(gtk::Align::Start);
    view_original.set_sensitive(false);
    page.append(&view_original);

    let compare_title = gtk::Label::new(Some("Differences from original"));
    compare_title.set_xalign(0.0);
    compare_title.add_css_class("heading");
    page.append(&compare_title);

    let compare_status = gtk::Label::new(None);
    compare_status.set_xalign(0.0);
    compare_status.set_wrap(true);
    compare_status.add_css_class("dim-label");
    page.append(&compare_status);

    let compare_list = gtk::ListBox::new();
    compare_list.set_selection_mode(gtk::SelectionMode::None);
    compare_list.add_css_class("boxed-list");
    let compare_scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&compare_list)
        .build();
    page.append(&compare_scroll);

    let fields = SourceFields {
        source,
        file_path,
        override_info,
        original_path,
        view_original,
        compare_status,
        compare_list,
    };
    (page, fields)
}

fn build_raw_page() -> (gtk::Box, RawPageFields) {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 12);
    page.set_margin_top(12);
    page.set_margin_bottom(12);
    page.set_margin_start(12);
    page.set_margin_end(12);

    let raw_title = gtk::Label::new(Some("Raw source preview"));
    raw_title.set_xalign(0.0);
    raw_title.add_css_class("heading");
    page.append(&raw_title);

    let raw_view = gtk::TextView::new();
    raw_view.set_editable(false);
    raw_view.set_cursor_visible(false);
    raw_view.set_monospace(true);
    raw_view.set_wrap_mode(gtk::WrapMode::None);
    let buffer = raw_view.buffer();
    let raw_scroll = gtk::ScrolledWindow::builder()
        .min_content_height(220)
        .vexpand(true)
        .child(&raw_view)
        .build();
    page.append(&raw_scroll);

    let extra_title = gtk::Label::new(Some("Extra and vendor keys"));
    extra_title.set_xalign(0.0);
    extra_title.add_css_class("heading");
    page.append(&extra_title);

    let extra_list = gtk::ListBox::new();
    extra_list.set_selection_mode(gtk::SelectionMode::None);
    extra_list.add_css_class("boxed-list");
    let extra_scroll = gtk::ScrolledWindow::builder()
        .min_content_height(180)
        .vexpand(true)
        .child(&extra_list)
        .build();
    page.append(&extra_scroll);

    let add_extra = gtk::Button::with_label("Add extra key");
    add_extra.set_halign(gtk::Align::Start);
    page.append(&add_extra);

    (
        page,
        RawPageFields {
            buffer,
            extra_list,
            add_extra,
        },
    )
}

fn build_action_bar() -> ActionFields {
    let status = gtk::Label::new(Some("Ready"));
    status.set_xalign(0.0);
    status.set_ellipsize(gtk::pango::EllipsizeMode::End);
    status.add_css_class("dim-label");

    let dirty_indicator = gtk::Label::new(None);
    dirty_indicator.set_xalign(1.0);
    dirty_indicator.set_visible(false);

    let open_location = gtk::Button::with_label("Open Location");
    let launch = gtk::Button::with_label("Launch");
    let revert = gtk::Button::with_label("Revert");
    let reset_override = gtk::Button::with_label("Reset Override");
    let delete_entry = gtk::Button::with_label("Delete");
    delete_entry.add_css_class("destructive-action");
    let validate = gtk::Button::with_label("Validate");
    let save = gtk::Button::with_label("Save");
    save.add_css_class("suggested-action");
    ActionFields {
        status,
        dirty_indicator,
        reset_override,
        delete_entry,
        open_location,
        launch,
        save,
        validate,
        revert,
    }
}

fn build_status_row(actions: &ActionFields) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.status.set_hexpand(true);
    row.append(&actions.status);
    row.append(&actions.dirty_indicator);
    row
}

fn build_action_buttons(actions: &ActionFields) -> gtk::Box {
    let container = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    container.append(&actions.open_location);
    container.append(&actions.launch);
    container.append(&actions.revert);
    container.append(&actions.reset_override);
    container.append(&actions.delete_entry);
    container.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    container.append(&actions.validate);
    container.append(&actions.save);
    container.set_halign(gtk::Align::End);
    container
}

fn combine_editor_widgets(parts: EditorParts) -> EditorWidgets {
    let EditorParts {
        general,
        advanced,
        source,
        locale,
        action,
        raw,
        actions,
        page_stack,
        view_stack,
    } = parts;

    EditorWidgets {
        id: general.id,
        name: general.name,
        generic_name: general.generic_name,
        comment: general.comment,
        exec: general.exec,
        icon: general.icon,
        icon_preview: general.icon_preview,
        import_icon: general.import_icon,
        browse_icon: general.browse_icon,
        categories: general.categories,
        keywords: general.keywords,
        mime_type: general.mime_type,
        entry_type: advanced.entry_type,
        version: advanced.version,
        try_exec: advanced.try_exec,
        path: advanced.path,
        url: advanced.url,
        only_show_in: advanced.only_show_in,
        not_show_in: advanced.not_show_in,
        implements: advanced.implements,
        terminal: advanced.terminal,
        no_display: advanced.no_display,
        hidden: advanced.hidden,
        startup_notify: advanced.startup_notify,
        dbus_activatable: advanced.dbus_activatable,
        prefers_non_default_gpu: advanced.prefers_non_default_gpu,
        single_main_window: advanced.single_main_window,
        source: source.source,
        file_path: source.file_path,
        override_info: source.override_info,
        original_path: source.original_path,
        view_original: source.view_original,
        compare_status: source.compare_status,
        compare_list: source.compare_list,
        locale_list: locale.list,
        locale_rows: Rc::new(RefCell::new(Vec::new())),
        add_locale: locale.add,
        action_list: action.list,
        action_rows: Rc::new(RefCell::new(Vec::new())),
        add_action: action.add,
        raw_buffer: raw.buffer,
        extra_list: raw.extra_list,
        extra_rows: Rc::new(RefCell::new(Vec::new())),
        add_extra: raw.add_extra,
        page_stack,
        status: actions.status,
        dirty_indicator: actions.dirty_indicator,
        view_stack,
        dirty_sink: Rc::new(RefCell::new(None)),
        reset_override: actions.reset_override,
        delete_entry: actions.delete_entry,
        open_location: actions.open_location,
        launch: actions.launch,
        save: actions.save,
        validate: actions.validate,
        revert: actions.revert,
    }
}

fn install_dirty_sink(state: &Rc<RefCell<AppState>>, widgets: &EditorWidgets) {
    let sink = DirtySink {
        state: Rc::downgrade(state),
        indicator: widgets.dirty_indicator.clone(),
        status: widgets.status.clone(),
        save: widgets.save.clone(),
    };
    *widgets.dirty_sink.borrow_mut() = Some(sink);
}

fn connect_dirty_tracking(widgets: &EditorWidgets) {
    for entry in [
        &widgets.id,
        &widgets.name,
        &widgets.generic_name,
        &widgets.comment,
        &widgets.exec,
        &widgets.icon,
        &widgets.categories,
        &widgets.keywords,
        &widgets.mime_type,
        &widgets.entry_type,
        &widgets.version,
        &widgets.try_exec,
        &widgets.path,
        &widgets.url,
        &widgets.only_show_in,
        &widgets.not_show_in,
        &widgets.implements,
    ] {
        connect_entry_dirty(widgets, entry);
    }

    for dropdown in [
        &widgets.terminal,
        &widgets.no_display,
        &widgets.hidden,
        &widgets.startup_notify,
        &widgets.dbus_activatable,
        &widgets.prefers_non_default_gpu,
        &widgets.single_main_window,
    ] {
        connect_dropdown_dirty(widgets, dropdown);
    }
}

fn connect_entry_dirty(widgets: &EditorWidgets, entry: &gtk::Entry) {
    connect_entry_dirty_sink(&widgets.dirty_sink, entry);
}

fn connect_entry_dirty_sink(sink: &Rc<RefCell<Option<DirtySink>>>, entry: &gtk::Entry) {
    let sink = sink.clone();
    entry.connect_changed(move |_| {
        mark_dirty_sink(&sink);
    });
}

fn connect_dropdown_dirty(widgets: &EditorWidgets, dropdown: &gtk::DropDown) {
    connect_dropdown_dirty_sink(&widgets.dirty_sink, dropdown);
}

fn connect_dropdown_dirty_sink(sink: &Rc<RefCell<Option<DirtySink>>>, dropdown: &gtk::DropDown) {
    let sink = sink.clone();
    dropdown.connect_selected_notify(move |_| {
        mark_dirty_sink(&sink);
    });
}

fn mark_dirty_sink(sink: &Rc<RefCell<Option<DirtySink>>>) {
    let current = sink.borrow().clone();
    let Some(current) = current else {
        return;
    };
    let Some(state) = current.state.upgrade() else {
        return;
    };
    if state.borrow().baseline.is_none() {
        return;
    }
    current.status.set_text("");
    current.status.set_tooltip_text(None);
    current.indicator.set_text("Unsaved changes");
    current.indicator.set_visible(true);
    current.save.set_sensitive(true);
}

fn clear_dirty_indicator(widgets: &EditorWidgets) {
    widgets.dirty_indicator.set_text("");
    widgets.dirty_indicator.set_visible(false);
    widgets.save.set_sensitive(false);
}

fn set_clean_baseline(state: &Rc<RefCell<AppState>>, widgets: &EditorWidgets) {
    state.borrow_mut().baseline = Some(capture_editor_snapshot(widgets));
    if state.borrow().persistence.needs_initial_save() {
        widgets.dirty_indicator.set_text("Not saved");
        widgets.dirty_indicator.set_visible(true);
        widgets.save.set_sensitive(true);
    } else {
        clear_dirty_indicator(widgets);
    }
}

fn clear_baseline(state: &Rc<RefCell<AppState>>, widgets: &EditorWidgets) {
    state.borrow_mut().baseline = None;
    clear_dirty_indicator(widgets);
}

fn has_unsaved_changes(state: &Rc<RefCell<AppState>>, widgets: &EditorWidgets) -> bool {
    let snapshot = capture_editor_snapshot(widgets);
    let borrowed = state.borrow();
    let dirty = borrowed.persistence.needs_initial_save()
        || borrowed
            .baseline
            .as_ref()
            .is_some_and(|baseline| baseline != &snapshot);
    if !dirty {
        clear_dirty_indicator(widgets);
    }
    dirty
}

fn capture_editor_snapshot(widgets: &EditorWidgets) -> EditorSnapshot {
    EditorSnapshot {
        id: widgets.id.text().to_string(),
        name: widgets.name.text().to_string(),
        generic_name: widgets.generic_name.text().to_string(),
        comment: widgets.comment.text().to_string(),
        exec: widgets.exec.text().to_string(),
        icon: widgets.icon.text().to_string(),
        categories: widgets.categories.text().to_string(),
        keywords: widgets.keywords.text().to_string(),
        mime_type: widgets.mime_type.text().to_string(),
        entry_type: widgets.entry_type.text().to_string(),
        version: widgets.version.text().to_string(),
        try_exec: widgets.try_exec.text().to_string(),
        path: widgets.path.text().to_string(),
        url: widgets.url.text().to_string(),
        only_show_in: widgets.only_show_in.text().to_string(),
        not_show_in: widgets.not_show_in.text().to_string(),
        implements: widgets.implements.text().to_string(),
        boolean_states: [
            widgets.terminal.selected(),
            widgets.no_display.selected(),
            widgets.hidden.selected(),
            widgets.startup_notify.selected(),
            widgets.dbus_activatable.selected(),
            widgets.prefers_non_default_gpu.selected(),
            widgets.single_main_window.selected(),
        ],
        localized_values: collect_locale_values(widgets),
        actions: collect_desktop_actions(widgets),
        extra_values: collect_extra_values(widgets),
    }
}

fn connect_search(state: &Rc<RefCell<AppState>>, widgets: &AppWidgets) {
    let state = state.clone();
    let widgets = widgets.clone();
    widgets.search.clone().connect_search_changed(move |_| {
        render_list(&state, &widgets);
    });
}

fn connect_filters(state: &Rc<RefCell<AppState>>, widgets: &AppWidgets) {
    let source_state = state.clone();
    let source_widgets = widgets.clone();
    widgets
        .source_filter
        .clone()
        .connect_selected_notify(move |_| {
            render_list(&source_state, &source_widgets);
        });

    let type_state = state.clone();
    let type_widgets = widgets.clone();
    widgets
        .type_filter
        .clone()
        .connect_selected_notify(move |_| {
            render_list(&type_state, &type_widgets);
        });
}

fn connect_selection(state: &Rc<RefCell<AppState>>, widgets: &AppWidgets) {
    let state = state.clone();
    let widgets = widgets.clone();
    widgets.list.clone().connect_row_selected(move |_, row| {
        if take_suppressed_selection(&state) {
            return;
        }
        let Some(row) = row else {
            return;
        };
        let Ok(index) = usize::try_from(row.index()) else {
            return;
        };
        let id = state.borrow().visible_ids.get(index).cloned();
        let Some(id) = id else {
            return;
        };
        if state.borrow().selected_id.as_deref() == Some(id.as_str()) {
            return;
        }

        if has_unsaved_changes(&state, &widgets.editor) {
            restore_current_selection(&state, &widgets);
            confirm_unsaved_changes(&state, &widgets, PendingAction::Select(id));
        } else {
            load_entry(&state, &widgets, &id);
        }
    });
}

fn take_suppressed_selection(state: &Rc<RefCell<AppState>>) -> bool {
    let mut borrowed = state.borrow_mut();
    if borrowed.selection_signal != SelectionSignal::SuppressNext {
        return false;
    }
    borrowed.selection_signal = SelectionSignal::Normal;
    true
}

fn restore_current_selection(state: &Rc<RefCell<AppState>>, widgets: &AppWidgets) {
    let selected_id = state.borrow().selected_id.clone();
    let Some(selected_id) = selected_id else {
        widgets.list.unselect_all();
        return;
    };
    select_row_silently(state, widgets, &selected_id);
}

fn select_row_silently(state: &Rc<RefCell<AppState>>, widgets: &AppWidgets, id: &str) {
    let index = state
        .borrow()
        .visible_ids
        .iter()
        .position(|visible_id| visible_id == id)
        .and_then(|index| i32::try_from(index).ok());
    let Some(index) = index else {
        widgets.list.unselect_all();
        return;
    };
    let Some(row) = widgets.list.row_at_index(index) else {
        return;
    };
    if widgets.list.selected_row().as_ref() == Some(&row) {
        return;
    }
    state.borrow_mut().selection_signal = SelectionSignal::SuppressNext;
    widgets.list.select_row(Some(&row));
}

fn connect_toolbar(state: &Rc<RefCell<AppState>>, widgets: &AppWidgets) {
    let reload_state = state.clone();
    let reload_widgets = widgets.clone();
    widgets.reload.clone().connect_clicked(move |_| {
        request_pending_action(&reload_state, &reload_widgets, PendingAction::Reload);
    });

    let new_state = state.clone();
    let new_widgets = widgets.clone();
    widgets.add.clone().connect_clicked(move |_| {
        request_pending_action(&new_state, &new_widgets, PendingAction::New);
    });

    let open_state = state.clone();
    let open_widgets = widgets.clone();
    widgets.open_file.clone().connect_clicked(move |_| {
        request_pending_action(&open_state, &open_widgets, PendingAction::OpenFile);
    });
}

fn connect_about(widgets: &AppWidgets) {
    let parent = widgets.window.downgrade();
    widgets.about.connect_clicked(move |_| {
        let Some(parent) = parent.upgrade() else {
            return;
        };
        let dialog = adw::AlertDialog::builder()
            .heading("DeskEntry")
            .body(format!("Version {} · hoek", env!("CARGO_PKG_VERSION")))
            .build();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let icon = gtk::Image::from_icon_name(crate::APP_ID);
        icon.set_pixel_size(64);
        let website = gtk::LinkButton::with_label(env!("CARGO_PKG_REPOSITORY"), "Project website");
        let releases = gtk::LinkButton::with_label(updates::RELEASES_URL, "GitHub Releases");
        let automatic = gtk::CheckButton::with_label("Check for updates on startup");
        automatic.set_active(updates::enabled());
        let check = gtk::Button::with_label("Check for Updates");
        let status = gtk::Label::new(None);
        status.set_wrap(true);
        status.add_css_class("dim-label");
        content.append(&icon);
        content.append(&website);
        content.append(&releases);
        content.append(&automatic);
        content.append(&check);
        content.append(&status);
        let preference_status = status.downgrade();
        automatic.connect_toggled(move |button| {
            if button.is_active() == updates::enabled() {
                return;
            }
            if updates::set_enabled(button.is_active()).is_err() {
                button.set_active(updates::enabled());
                if let Some(status) = preference_status.upgrade() {
                    status.set_text("Could not save update preference.");
                }
            }
        });
        let result_status = status.downgrade();
        check.connect_clicked(move |button| {
            let Some(status) = result_status.upgrade() else {
                return;
            };
            button.set_sensitive(false);
            status.set_text("Checking for updates…");
            let button = button.downgrade();
            let status = status.downgrade();
            let _task = gtk::glib::MainContext::default().spawn_local(async move {
                let result = gio::spawn_blocking(updates::check).await.ok().flatten();
                if let Some(status) = status.upgrade() {
                    let message = match result {
                        Some(release) if release.newer => {
                            format!("New version available: v{}", release.version)
                        }
                        Some(release) => {
                            format!("No newer release. Latest: v{}", release.version)
                        }
                        None => "Could not check for updates. Check your connection or curl installation."
                            .to_string(),
                    };
                    status.set_text(&message);
                }
                if let Some(button) = button.upgrade() {
                    button.set_sensitive(true);
                }
            });
        });
        dialog.set_extra_child(Some(&content));
        dialog.add_response("close", "OK");
        dialog.set_close_response("close");
        dialog.set_default_response(Some("close"));
        dialog.present(Some(&parent));
    });
}

fn check_startup_update(window: &adw::ApplicationWindow) {
    if !updates::should_check_at_startup() {
        return;
    }
    let window = window.downgrade();
    let _task = gtk::glib::MainContext::default().spawn_local(async move {
        let Some(release) = gio::spawn_blocking(updates::check).await.ok().flatten() else {
            return;
        };
        if release.newer
            && let Some(window) = window.upgrade()
            && gtk::prelude::WidgetExt::is_visible(&window)
        {
            let dialog = adw::AlertDialog::builder()
                .heading("Update Available")
                .body(format!(
                    "DeskEntry v{} is available. You are using v{}.",
                    release.version,
                    env!("CARGO_PKG_VERSION")
                ))
                .build();
            let link = gtk::LinkButton::with_label(updates::RELEASES_URL, "Open GitHub Releases");
            dialog.set_extra_child(Some(&link));
            dialog.add_response("close", "OK");
            dialog.set_close_response("close");
            dialog.set_default_response(Some("close"));
            dialog.present(Some(&window));
        }
    });
}

fn connect_window_actions(
    app: &adw::Application,
    state: &Rc<RefCell<AppState>>,
    widgets: &AppWidgets,
) {
    let save = gio::SimpleAction::new("save", None);
    let save_state = state.clone();
    let save_widgets = widgets.clone();
    save.connect_activate(move |_, _| {
        if let Err(error) = save_current_entry(&save_state, &save_widgets) {
            show_editor_error(&save_widgets, "Save Failed", &error);
        }
    });
    widgets.window.add_action(&save);

    let new_entry = gio::SimpleAction::new("new", None);
    let new_state = state.clone();
    let new_widgets = widgets.clone();
    new_entry.connect_activate(move |_, _| {
        request_pending_action(&new_state, &new_widgets, PendingAction::New);
    });
    widgets.window.add_action(&new_entry);

    let reload = gio::SimpleAction::new("reload", None);
    let reload_state = state.clone();
    let reload_widgets = widgets.clone();
    reload.connect_activate(move |_, _| {
        request_pending_action(&reload_state, &reload_widgets, PendingAction::Reload);
    });
    widgets.window.add_action(&reload);

    let open_file = gio::SimpleAction::new("open-file", None);
    let open_state = state.clone();
    let open_widgets = widgets.clone();
    open_file.connect_activate(move |_, _| {
        request_pending_action(&open_state, &open_widgets, PendingAction::OpenFile);
    });
    widgets.window.add_action(&open_file);

    let search = gio::SimpleAction::new("search", None);
    let search_entry = widgets.search.clone();
    search.connect_activate(move |_, _| {
        search_entry.grab_focus();
    });
    widgets.window.add_action(&search);

    app.set_accels_for_action("win.save", &["<Primary>s"]);
    app.set_accels_for_action("win.new", &["<Primary>n"]);
    app.set_accels_for_action("win.open-file", &["<Primary>o"]);
    app.set_accels_for_action("win.reload", &["<Primary>r"]);
    app.set_accels_for_action("win.search", &["<Primary>f"]);
}

fn connect_close_guard(state: &Rc<RefCell<AppState>>, widgets: &AppWidgets) {
    let state = state.clone();
    let widgets = widgets.clone();
    widgets.window.clone().connect_close_request(move |_| {
        if state.borrow().close_guard == CloseGuard::Allow {
            return gtk::glib::Propagation::Proceed;
        }
        if !has_unsaved_changes(&state, &widgets.editor) {
            return gtk::glib::Propagation::Proceed;
        }
        confirm_unsaved_changes(&state, &widgets, PendingAction::Close);
        gtk::glib::Propagation::Stop
    });
}

fn request_pending_action(
    state: &Rc<RefCell<AppState>>,
    widgets: &AppWidgets,
    action: PendingAction,
) {
    if has_unsaved_changes(state, &widgets.editor) {
        confirm_unsaved_changes(state, widgets, action);
    } else {
        perform_pending_action(state, widgets, action);
    }
}

fn confirm_unsaved_changes(
    state: &Rc<RefCell<AppState>>,
    widgets: &AppWidgets,
    action: PendingAction,
) {
    let dialog = gtk::AlertDialog::builder()
        .message("Save changes before continuing?")
        .detail("Unsaved changes will be lost if you discard them.")
        .modal(true)
        .build();
    dialog.set_buttons(&["Cancel", "Discard", "Save"]);
    dialog.set_cancel_button(0);
    dialog.set_default_button(0);

    let state = state.clone();
    let widgets = widgets.clone();
    let parent = widgets.window.clone();
    dialog.choose(
        Some(&parent),
        None::<&gio::Cancellable>,
        move |result| match result {
            Ok(1) => perform_pending_action(&state, &widgets, action),
            Ok(2) => match save_current_entry(&state, &widgets) {
                Ok(()) => perform_pending_action(&state, &widgets, action),
                Err(error) => show_editor_error(&widgets, "Save Failed", &error),
            },
            _ => {}
        },
    );
}

fn perform_pending_action(
    state: &Rc<RefCell<AppState>>,
    widgets: &AppWidgets,
    action: PendingAction,
) {
    match action {
        PendingAction::Select(id) => {
            select_row_silently(state, widgets, &id);
            load_entry(state, widgets, &id);
        }
        PendingAction::Reload => {
            let selected_id = state.borrow().selected_id.clone();
            reload_entries(state, widgets, selected_id.as_deref());
        }
        PendingAction::New => start_new_entry(state, widgets),
        PendingAction::OpenFile => open_desktop_file_dialog(state, widgets),
        PendingAction::Close => {
            state.borrow_mut().close_guard = CloseGuard::Allow;
            widgets.window.close();
        }
    }
}

fn connect_editor_actions(state: &Rc<RefCell<AppState>>, widgets: &AppWidgets) {
    connect_save(state, widgets);
    connect_validate(state, widgets);
    connect_revert(state, widgets);
    connect_reset_override(state, widgets);
    connect_delete_entry(state, widgets);
    connect_launch(state, widgets);
    connect_open_location(state, widgets);
    connect_view_original(state, widgets);
}

fn connect_save(state: &Rc<RefCell<AppState>>, widgets: &AppWidgets) {
    let state = state.clone();
    let widgets = widgets.clone();
    widgets.editor.save.clone().connect_clicked(move |_| {
        if let Err(error) = save_current_entry(&state, &widgets) {
            show_editor_error(&widgets, "Save Failed", &error);
        }
    });
}

fn save_current_entry(state: &Rc<RefCell<AppState>>, widgets: &AppWidgets) -> Result<(), String> {
    let Some(mut document) = state.borrow().document.clone() else {
        return Err("Select or create a desktop entry first".to_string());
    };
    let needs_initial_save = state.borrow().persistence.needs_initial_save();
    if !needs_initial_save && !has_unsaved_changes(state, &widgets.editor) {
        set_status(&widgets.editor, "No changes to save");
        return Ok(());
    }
    apply_widgets_to_document(&widgets.editor, &mut document)?;
    let id = current_save_id(state, &widgets.editor);
    let id_exists = {
        let borrowed = state.borrow();
        borrowed.persistence == EntryPersistence::Unsaved
            && borrowed.groups.iter().any(|group| group.id == id)
    };
    if id_exists {
        return Err(
            "A desktop entry with this ID already exists; edit the existing entry instead"
                .to_string(),
        );
    }

    let user_path = {
        let borrowed = state.borrow();
        if borrowed.persistence == EntryPersistence::Existing {
            borrowed
                .groups
                .iter()
                .find(|group| group.id == id)
                .and_then(EntryGroup::effective)
                .filter(|variant| variant.source == SourceKind::User)
                .map(|variant| variant.path.clone())
        } else {
            None
        }
    };
    let result = storage::save_user_override(&id, &document.serialize(), user_path.as_deref())?;
    update_state_after_save(state, &id, document, result.path);
    reload_entries(state, widgets, Some(&id));
    set_success_status(
        &widgets.editor,
        "Saved user override",
        result.warning.as_deref(),
    );
    Ok(())
}

fn connect_validate(state: &Rc<RefCell<AppState>>, widgets: &AppWidgets) {
    let state = state.clone();
    let widgets = widgets.clone();
    widgets.editor.validate.clone().connect_clicked(move |_| {
        let Some(mut document) = state.borrow().document.clone() else {
            set_status(&widgets.editor, "Select or create a desktop entry first");
            return;
        };
        if let Err(error) = apply_widgets_to_document(&widgets.editor, &mut document) {
            show_editor_error(&widgets, "Validation Failed", &error);
            return;
        }
        match storage::validate_contents(&document.serialize()) {
            Ok(message) => set_status(&widgets.editor, &message),
            Err(error) => show_editor_error(&widgets, "Validation Failed", &error),
        }
    });
}

fn revert_selected_entry(state: &RefCell<AppState>, load: impl FnOnce(&str)) -> bool {
    let selected_id = state.borrow().selected_id.clone();
    if let Some(id) = selected_id {
        load(&id);
        true
    } else {
        false
    }
}

fn connect_revert(state: &Rc<RefCell<AppState>>, widgets: &AppWidgets) {
    let state = state.clone();
    let widgets = widgets.clone();
    widgets.editor.revert.clone().connect_clicked(move |_| {
        if !has_unsaved_changes(&state, &widgets.editor) {
            set_status(&widgets.editor, "There are no unsaved changes to revert");
            return;
        }

        let dialog = confirmation_dialog(
            "Revert unsaved changes?",
            "All changes made since the entry was loaded or last saved will be discarded.",
            "Revert",
        );
        let state = state.clone();
        let widgets = widgets.clone();
        let parent = widgets.window.clone();
        dialog.choose(Some(&parent), None::<&gio::Cancellable>, move |result| {
            if !matches!(result, Ok(1)) {
                return;
            }
            if !revert_selected_entry(&state, |id| load_entry(&state, &widgets, id)) {
                let (document, baseline_id) = {
                    let borrowed = state.borrow();
                    (
                        borrowed.document.clone(),
                        borrowed
                            .baseline
                            .as_ref()
                            .map(|baseline| baseline.id.clone()),
                    )
                };
                if let Some(document) = document {
                    apply_document(&widgets.editor, &document);
                    if let Some(baseline_id) = baseline_id {
                        widgets.editor.id.set_text(&baseline_id);
                    }
                    set_clean_baseline(&state, &widgets.editor);
                    refresh_raw_preview(&state, &widgets.editor);
                }
            }
            set_status(&widgets.editor, "Reverted unsaved changes");
        });
    });
}

fn connect_reset_override(state: &Rc<RefCell<AppState>>, widgets: &AppWidgets) {
    let state = state.clone();
    let widgets = widgets.clone();
    widgets
        .editor
        .reset_override
        .clone()
        .connect_clicked(move |_| {
            let (path, id) = {
                let borrowed = state.borrow();
                (
                    borrowed.effective_path.clone(),
                    borrowed.selected_id.clone(),
                )
            };
            let Some(path) = path else {
                return;
            };

            let dialog = confirmation_dialog(
                "Reset this user override?",
                "The user override will be removed and the next entry in XDG precedence order will become active.",
                "Reset Override",
            );
            let state = state.clone();
            let widgets = widgets.clone();
            let parent = widgets.window.clone();
            dialog.choose(Some(&parent), None::<&gio::Cancellable>, move |result| {
                if !matches!(result, Ok(1)) {
                    return;
                }
                match storage::remove_user_override(&path) {
                    Ok(warning) => {
                        reload_entries(&state, &widgets, id.as_deref());
                        set_success_status(
                            &widgets.editor,
                            "User override removed",
                            warning.as_deref(),
                        );
                    }
                    Err(error) => set_status(&widgets.editor, &error),
                }
            });
        });
}

fn connect_delete_entry(state: &Rc<RefCell<AppState>>, widgets: &AppWidgets) {
    let state = state.clone();
    let widgets = widgets.clone();
    widgets
        .editor
        .delete_entry
        .clone()
        .connect_clicked(move |_| {
            let Some(path) = state.borrow().effective_path.clone() else {
                return;
            };
            confirm_user_entry_deletion(state.clone(), widgets.clone(), path);
        });
}

fn confirm_user_entry_deletion(state: Rc<RefCell<AppState>>, widgets: AppWidgets, path: PathBuf) {
    let dialog = confirmation_dialog(
        "Delete this user desktop entry?",
        &format!("This will remove {}.", path.display()),
        "Continue",
    );
    let parent = widgets.window.clone();
    dialog.choose(Some(&parent), None::<&gio::Cancellable>, move |result| {
        if matches!(result, Ok(1)) {
            confirm_user_entry_deletion_final(state, widgets, path);
        }
    });
}

fn confirm_user_entry_deletion_final(
    state: Rc<RefCell<AppState>>,
    widgets: AppWidgets,
    path: PathBuf,
) {
    let dialog = confirmation_dialog(
        "Delete the desktop entry now?",
        "The active .desktop file will be removed from your XDG user applications directory. Existing .bak backups are left untouched.",
        "Delete",
    );
    let parent = widgets.window.clone();
    dialog.choose(Some(&parent), None::<&gio::Cancellable>, move |result| {
        if !matches!(result, Ok(1)) {
            return;
        }
        match storage::delete_user_entry(&path) {
            Ok(warning) => finish_user_entry_deletion(&state, &widgets, warning.as_deref()),
            Err(error) => set_status(&widgets.editor, &error),
        }
    });
}

fn finish_user_entry_deletion(
    state: &Rc<RefCell<AppState>>,
    widgets: &AppWidgets,
    warning: Option<&str>,
) {
    {
        let mut borrowed = state.borrow_mut();
        borrowed.selected_id = None;
        borrowed.document = None;
        borrowed.effective_path = None;
        borrowed.baseline = None;
        borrowed.persistence = EntryPersistence::Existing;
    }
    widgets.list.unselect_all();
    reload_entries(state, widgets, None);
    set_editor_sensitive(&widgets.editor, false);
    clear_baseline(state, &widgets.editor);
    set_success_status(&widgets.editor, "User desktop entry deleted", warning);
}

fn confirmation_dialog(message: &str, detail: &str, confirm_label: &str) -> gtk::AlertDialog {
    let dialog = gtk::AlertDialog::builder()
        .message(message)
        .detail(detail)
        .modal(true)
        .build();
    dialog.set_buttons(&["Cancel", confirm_label]);
    dialog.set_cancel_button(0);
    dialog.set_default_button(0);
    dialog
}

fn connect_launch(state: &Rc<RefCell<AppState>>, widgets: &AppWidgets) {
    let state = state.clone();
    let editor = widgets.editor.clone();
    widgets.editor.launch.clone().connect_clicked(move |_| {
        let Some(path) = state.borrow().effective_path.clone() else {
            set_status(&editor, "Save the entry before launching it");
            return;
        };
        if let Err(error) = storage::launch_desktop_file(&path) {
            set_status(&editor, &error);
        }
    });
}

fn connect_open_location(state: &Rc<RefCell<AppState>>, widgets: &AppWidgets) {
    let state = state.clone();
    let editor = widgets.editor.clone();
    widgets
        .editor
        .open_location
        .clone()
        .connect_clicked(move |_| {
            let Some(path) = state.borrow().effective_path.clone() else {
                return;
            };
            if let Err(error) = storage::open_file_location(&path) {
                set_status(&editor, &error);
            }
        });
}

fn connect_view_original(state: &Rc<RefCell<AppState>>, widgets: &AppWidgets) {
    let state = state.clone();
    let widgets = widgets.clone();
    widgets
        .editor
        .view_original
        .clone()
        .connect_clicked(move |_| {
            let Some(original) = original_variant(&state) else {
                set_status(&widgets.editor, "No original desktop entry is available");
                return;
            };
            show_source_window(
                &widgets.window,
                "Original Desktop Entry",
                &original.document.serialize(),
            );
        });
}

fn show_source_window(parent: &adw::ApplicationWindow, title: &str, contents: &str) {
    show_text_window(parent, title, contents, gtk::WrapMode::None);
}

fn show_editor_error(widgets: &AppWidgets, title: &str, error: &str) {
    set_status(&widgets.editor, title);
    widgets.editor.status.set_tooltip_text(Some(error));
    show_text_window(&widgets.window, title, error, gtk::WrapMode::WordChar);
}

fn show_text_window(
    parent: &adw::ApplicationWindow,
    title: &str,
    contents: &str,
    wrap_mode: gtk::WrapMode,
) {
    let window = adw::Window::builder()
        .title(title)
        .default_width(760)
        .default_height(620)
        .modal(true)
        .transient_for(parent)
        .build();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&adw::HeaderBar::new());
    let view = gtk::TextView::new();
    view.set_editable(false);
    view.set_cursor_visible(false);
    view.set_monospace(true);
    view.set_wrap_mode(wrap_mode);
    view.set_top_margin(12);
    view.set_bottom_margin(12);
    view.set_left_margin(12);
    view.set_right_margin(12);
    view.buffer().set_text(contents);
    let scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hexpand(true)
        .child(&view)
        .build();
    root.append(&scroll);
    window.set_content(Some(&root));
    window.present();
}

fn connect_icon_import(widgets: &AppWidgets) {
    let window = widgets.window.clone();
    let editor = widgets.editor.clone();
    widgets
        .editor
        .import_icon
        .clone()
        .connect_clicked(move |_| {
            let dialog = gtk::FileDialog::builder()
                .title("Import Icon")
                .modal(true)
                .build();
            let editor = editor.clone();
            let parent = window.clone();
            dialog.open(Some(&window), None::<&gio::Cancellable>, move |result| {
                let Ok(file) = result else {
                    return;
                };
                let Some(path) = file.path() else {
                    let error = "Selected icon has no local filesystem path";
                    set_status(&editor, error);
                    show_text_window(
                        &parent,
                        "Icon Import Failed",
                        error,
                        gtk::WrapMode::WordChar,
                    );
                    return;
                };
                match storage::import_icon(&path) {
                    Ok(result) => {
                        editor.icon.set_text(&result.name);
                        set_success_status(
                            &editor,
                            "Icon imported into the user hicolor theme",
                            result.warning.as_deref(),
                        );
                    }
                    Err(error) => {
                        set_status(&editor, "Icon Import Failed");
                        let message = format!("{}\n\n{error}", path.display());
                        show_text_window(
                            &parent,
                            "Icon Import Failed",
                            &message,
                            gtk::WrapMode::WordChar,
                        );
                    }
                }
            });
        });
}

fn start_new_entry(state: &Rc<RefCell<AppState>>, widgets: &AppWidgets) {
    widgets.list.unselect_all();
    let document = DesktopDocument::new_application();
    {
        let mut borrowed = state.borrow_mut();
        borrowed.selected_id = None;
        borrowed.document = Some(document.clone());
        borrowed.effective_path = None;
        borrowed.baseline = None;
        borrowed.persistence = EntryPersistence::Unsaved;
    }
    apply_document(&widgets.editor, &document);
    set_editor_sensitive(&widgets.editor, true);
    widgets.editor.id.set_text(&next_new_desktop_id(state));
    widgets.editor.id.set_sensitive(true);
    widgets.editor.source.set_text("User");
    widgets.editor.file_path.set_text("Not saved yet");
    widgets
        .editor
        .override_info
        .set_text("New user desktop entry");
    widgets.editor.original_path.set_text("");
    widgets.editor.view_original.set_sensitive(false);
    widgets
        .editor
        .compare_status
        .set_text("New entries have no original comparison.");
    clear_list(&widgets.editor.compare_list);
    widgets.editor.reset_override.set_sensitive(false);
    widgets.editor.delete_entry.set_sensitive(false);
    widgets.editor.open_location.set_sensitive(false);
    widgets.editor.launch.set_sensitive(false);
    set_clean_baseline(state, &widgets.editor);
    refresh_raw_preview(state, &widgets.editor);
    widgets.editor.name.grab_focus();
    set_status(&widgets.editor, "New desktop entry");
}

fn next_new_desktop_id(state: &Rc<RefCell<AppState>>) -> String {
    let borrowed = state.borrow();
    if !borrowed
        .groups
        .iter()
        .any(|group| group.id == "new-application.desktop")
    {
        return "new-application.desktop".to_string();
    }

    match (2..=9999)
        .map(|suffix| format!("new-application-{suffix}.desktop"))
        .find(|candidate| !borrowed.groups.iter().any(|group| group.id == *candidate))
    {
        Some(candidate) => candidate,
        None => "new-application-custom.desktop".to_string(),
    }
}

fn current_save_id(state: &Rc<RefCell<AppState>>, widgets: &EditorWidgets) -> String {
    let borrowed = state.borrow();
    if borrowed.persistence == EntryPersistence::Unsaved {
        widgets.id.text().to_string()
    } else {
        borrowed
            .selected_id
            .clone()
            .unwrap_or_else(|| widgets.id.text().to_string())
    }
}

fn update_state_after_save(
    state: &Rc<RefCell<AppState>>,
    id: &str,
    document: DesktopDocument,
    path: PathBuf,
) {
    let mut borrowed = state.borrow_mut();
    borrowed.document = Some(document);
    borrowed.selected_id = Some(id.to_string());
    borrowed.effective_path = Some(path);
    borrowed.persistence = EntryPersistence::Existing;
}

fn reload_entries(state: &Rc<RefCell<AppState>>, widgets: &AppWidgets, select_id: Option<&str>) {
    state.borrow_mut().groups = scanner::scan_entries();
    render_list(state, widgets);

    if let Some(id) = select_id
        && state.borrow().groups.iter().any(|group| group.id == id)
    {
        select_row_silently(state, widgets, id);
        load_entry(state, widgets, id);
        return;
    }

    let selected_exists = {
        let borrowed = state.borrow();
        borrowed
            .selected_id
            .as_ref()
            .is_some_and(|selected| borrowed.groups.iter().any(|group| &group.id == selected))
    };
    if !selected_exists {
        clear_editor_selection(state, widgets);
    }

    if state.borrow().groups.is_empty() {
        set_status(&widgets.editor, "No desktop entries found");
    }
}

fn clear_editor_selection(state: &Rc<RefCell<AppState>>, widgets: &AppWidgets) {
    {
        let mut borrowed = state.borrow_mut();
        borrowed.selected_id = None;
        borrowed.document = None;
        borrowed.effective_path = None;
        borrowed.baseline = None;
        borrowed.persistence = EntryPersistence::Existing;
    }
    widgets.list.unselect_all();
    clear_list(&widgets.editor.compare_list);
    widgets.editor.original_path.set_text("");
    widgets.editor.compare_status.set_text("");
    widgets.editor.raw_buffer.set_text("");
    set_editor_sensitive(&widgets.editor, false);
    clear_dirty_indicator(&widgets.editor);
}

fn render_list(state: &Rc<RefCell<AppState>>, widgets: &AppWidgets) {
    clear_list(&widgets.list);
    let query = widgets.search.text().trim().to_lowercase();
    let entries = visible_entries(
        state,
        &query,
        widgets.source_filter.selected(),
        widgets.type_filter.selected(),
    );
    let mut visible_ids = Vec::with_capacity(entries.len());

    for entry in entries {
        append_application_row(&widgets.list, &entry);
        visible_ids.push(entry.id);
    }

    let has_entries = !visible_ids.is_empty();
    state.borrow_mut().visible_ids = visible_ids;
    widgets
        .list_stack
        .set_visible_child_name(if has_entries { "list" } else { "empty" });
    restore_current_selection(state, widgets);
}

fn visible_entries(
    state: &Rc<RefCell<AppState>>,
    query: &str,
    source_filter: u32,
    type_filter: u32,
) -> Vec<VisibleEntry> {
    state
        .borrow()
        .groups
        .iter()
        .filter_map(|group| {
            let effective = group.effective()?;
            let name = group.display_name();
            let entry_type = effective.document.get_main("Type").unwrap_or_default();
            let matches_query = query.is_empty()
                || name.to_lowercase().contains(query)
                || group.id.to_lowercase().contains(query);
            if !matches_query
                || !source_matches_filter(source_filter, effective.source)
                || !type_matches_filter(type_filter, entry_type)
            {
                return None;
            }
            Some(VisibleEntry {
                id: group.id.clone(),
                name,
                icon: effective
                    .document
                    .localized_main("Icon")
                    .unwrap_or_default()
                    .to_string(),
                entry_type: entry_type.to_string(),
                source: effective.source,
            })
        })
        .collect()
}

fn source_matches_filter(filter: u32, source: SourceKind) -> bool {
    match filter {
        0 => true,
        1 => source == SourceKind::User,
        2 => source == SourceKind::System,
        3 => matches!(source, SourceKind::FlatpakUser | SourceKind::FlatpakSystem),
        4 => source == SourceKind::Snap,
        5 => source == SourceKind::CustomXdg,
        _ => false,
    }
}

fn type_matches_filter(filter: u32, entry_type: &str) -> bool {
    match filter {
        0 => true,
        1 => entry_type == "Application",
        2 => entry_type == "Link",
        3 => entry_type == "Directory",
        4 => !matches!(entry_type, "Application" | "Link" | "Directory"),
        _ => false,
    }
}

fn append_application_row(list: &gtk::ListBox, entry: &VisibleEntry) {
    let row = gtk::ListBoxRow::new();
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    content.set_margin_top(8);
    content.set_margin_bottom(8);
    content.set_margin_start(10);
    content.set_margin_end(10);

    let image = gtk::Image::new();
    image.set_pixel_size(32);
    set_list_icon(&image, &entry.icon);

    let labels = gtk::Box::new(gtk::Orientation::Vertical, 2);
    labels.set_hexpand(true);
    let title = gtk::Label::new(Some(&entry.name));
    title.set_xalign(0.0);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let entry_type = if entry.entry_type.is_empty() {
        "Unknown"
    } else {
        entry.entry_type.as_str()
    };
    let subtitle = gtk::Label::new(Some(&format!(
        "{} · {} · {}",
        entry.source.label(),
        entry_type,
        entry.id
    )));
    subtitle.set_xalign(0.0);
    subtitle.set_ellipsize(gtk::pango::EllipsizeMode::End);
    subtitle.add_css_class("dim-label");
    labels.append(&title);
    labels.append(&subtitle);

    content.append(&image);
    content.append(&labels);
    row.set_child(Some(&content));
    list.append(&row);
}

fn set_list_icon(image: &gtk::Image, icon: &str) {
    let icon = icon.trim();
    image.set_use_fallback(true);
    if icon.is_empty() || Path::new(icon).is_absolute() {
        image.set_icon_name(Some("application-x-executable-symbolic"));
    } else {
        image.set_icon_name(Some(icon));
    }
}

fn clear_list(list: &gtk::ListBox) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
}

fn load_entry(state: &Rc<RefCell<AppState>>, widgets: &AppWidgets, id: &str) {
    let Some(selected) = selected_entry(state, id) else {
        return;
    };
    {
        let mut borrowed = state.borrow_mut();
        borrowed.selected_id = Some(selected.id.clone());
        borrowed.document = Some(selected.effective.document.clone());
        borrowed.effective_path = Some(selected.effective.path.clone());
        borrowed.baseline = None;
        borrowed.persistence = EntryPersistence::Existing;
    }

    apply_document(&widgets.editor, &selected.effective.document);
    set_editor_sensitive(&widgets.editor, true);
    widgets.editor.id.set_text(&selected.id);
    widgets.editor.id.set_sensitive(false);
    widgets
        .editor
        .source
        .set_text(selected.effective.source.label());
    widgets
        .editor
        .file_path
        .set_text(&selected.effective.path.to_string_lossy());
    widgets.editor.open_location.set_sensitive(true);
    widgets.editor.launch.set_sensitive(true);
    set_override_state(
        &widgets.editor,
        selected.effective.source,
        selected.variant_count,
        selected.lower_path.as_deref(),
    );
    set_clean_baseline(state, &widgets.editor);
    refresh_raw_preview(state, &widgets.editor);
    refresh_source_comparison(state, widgets);
    set_status(&widgets.editor, "Loaded desktop entry");
}

fn selected_entry(state: &Rc<RefCell<AppState>>, id: &str) -> Option<SelectedEntry> {
    let borrowed = state.borrow();
    let group = borrowed.groups.iter().find(|group| group.id == id)?;
    let effective = group.effective()?.clone();
    let lower_path = group
        .variants
        .get(1)
        .map(|variant| variant.path.to_string_lossy().to_string());
    Some(SelectedEntry {
        id: group.id.clone(),
        effective,
        variant_count: group.variants.len(),
        lower_path,
    })
}

fn set_override_state(
    widgets: &EditorWidgets,
    source: SourceKind,
    variant_count: usize,
    lower_path: Option<&str>,
) {
    let has_user_override = source == SourceKind::User && variant_count > 1;
    let is_user_owned = source == SourceKind::User && variant_count == 1;
    widgets.reset_override.set_sensitive(has_user_override);
    widgets.delete_entry.set_sensitive(is_user_owned);
    widgets
        .reset_override
        .set_tooltip_text(Some(if has_user_override {
            "Remove this user override and restore the next XDG entry"
        } else {
            "Only user overrides can be reset"
        }));
    widgets
        .delete_entry
        .set_tooltip_text(Some(if is_user_owned {
            "Delete this user-owned desktop entry"
        } else {
            "Package-managed entries cannot be deleted by DeskEntry"
        }));

    if has_user_override {
        let lower = lower_path.unwrap_or("packaged entry");
        widgets
            .override_info
            .set_text(&format!("User override of {lower}"));
    } else if source == SourceKind::User {
        widgets.override_info.set_text("User-owned entry");
    } else {
        widgets.override_info.set_text(
            "Saving changes creates a user override in the XDG user applications directory",
        );
    }
}

fn apply_document(widgets: &EditorWidgets, document: &DesktopDocument) {
    set_entry(&widgets.name, document.get_main("Name"));
    set_entry(&widgets.generic_name, document.get_main("GenericName"));
    set_entry(&widgets.comment, document.get_main("Comment"));
    set_entry(&widgets.exec, document.get_main("Exec"));
    set_entry(&widgets.icon, document.get_main("Icon"));
    set_entry(&widgets.categories, document.get_main("Categories"));
    set_entry(&widgets.keywords, document.get_main("Keywords"));
    set_entry(&widgets.mime_type, document.get_main("MimeType"));
    set_entry(&widgets.entry_type, document.get_main("Type"));
    set_entry(&widgets.version, document.get_main("Version"));
    set_entry(&widgets.try_exec, document.get_main("TryExec"));
    set_entry(&widgets.path, document.get_main("Path"));
    set_entry(&widgets.url, document.get_main("URL"));
    set_entry(&widgets.only_show_in, document.get_main("OnlyShowIn"));
    set_entry(&widgets.not_show_in, document.get_main("NotShowIn"));
    set_entry(&widgets.implements, document.get_main("Implements"));
    apply_optional_boolean(&widgets.terminal, document.get_main("Terminal"));
    apply_optional_boolean(&widgets.no_display, document.get_main("NoDisplay"));
    apply_optional_boolean(&widgets.hidden, document.get_main("Hidden"));
    apply_optional_boolean(&widgets.startup_notify, document.get_main("StartupNotify"));
    apply_optional_boolean(
        &widgets.dbus_activatable,
        document.get_main("DBusActivatable"),
    );
    apply_optional_boolean(
        &widgets.prefers_non_default_gpu,
        document.get_main("PrefersNonDefaultGPU"),
    );
    apply_optional_boolean(
        &widgets.single_main_window,
        document.get_main("SingleMainWindow"),
    );
    clear_locale_rows(widgets);
    for value in document.localized_values() {
        add_locale_row(widgets, Some(value));
    }
    clear_action_rows(widgets);
    for action in document.desktop_actions() {
        add_action_row(widgets, Some(action));
    }
    clear_extra_rows(widgets);
    for value in document.extra_key_values() {
        add_extra_row(widgets, Some(value));
    }
    widgets.raw_buffer.set_text(&document.serialize());
    update_type_aware_fields(widgets);
}

fn apply_widgets_to_document(
    widgets: &EditorWidgets,
    document: &mut DesktopDocument,
) -> Result<(), String> {
    let localized_values = collect_locale_values(widgets);

    set_required(document, "Type", widgets.entry_type.text().as_str());
    set_optional(document, "Version", widgets.version.text().as_str());
    set_required(document, "Name", widgets.name.text().as_str());
    set_localizable_optional(
        document,
        "GenericName",
        widgets.generic_name.text().as_str(),
        &localized_values,
    );
    set_localizable_optional(
        document,
        "Comment",
        widgets.comment.text().as_str(),
        &localized_values,
    );
    set_optional(document, "Exec", widgets.exec.text().as_str());
    set_localizable_optional(
        document,
        "Icon",
        widgets.icon.text().as_str(),
        &localized_values,
    );
    set_list(document, "Categories", widgets.categories.text().as_str());
    set_localizable_list(
        document,
        "Keywords",
        widgets.keywords.text().as_str(),
        &localized_values,
    );
    set_list(document, "MimeType", widgets.mime_type.text().as_str());
    set_optional(document, "TryExec", widgets.try_exec.text().as_str());
    set_optional(document, "Path", widgets.path.text().as_str());
    set_optional(document, "URL", widgets.url.text().as_str());
    set_list(document, "OnlyShowIn", widgets.only_show_in.text().as_str());
    set_list(document, "NotShowIn", widgets.not_show_in.text().as_str());
    set_list(document, "Implements", widgets.implements.text().as_str());
    set_optional_boolean(document, "Terminal", &widgets.terminal);
    set_optional_boolean(document, "NoDisplay", &widgets.no_display);
    set_optional_boolean(document, "Hidden", &widgets.hidden);
    set_optional_boolean(document, "StartupNotify", &widgets.startup_notify);
    set_optional_boolean(document, "DBusActivatable", &widgets.dbus_activatable);
    set_optional_boolean(
        document,
        "PrefersNonDefaultGPU",
        &widgets.prefers_non_default_gpu,
    );
    set_optional_boolean(document, "SingleMainWindow", &widgets.single_main_window);
    document.replace_localized_values(&localized_values)?;
    let actions = if matches!(widgets.entry_type.text().as_str(), "Link" | "Directory") {
        Vec::new()
    } else {
        collect_desktop_actions(widgets)
    };
    document.replace_actions_and_extra(&actions, &collect_extra_values(widgets))?;
    document.prune_type_specific_fields();
    Ok(())
}

fn set_entry(entry: &gtk::Entry, value: Option<&str>) {
    entry.set_text(value.unwrap_or_default());
}

fn set_required(document: &mut DesktopDocument, key: &str, value: &str) {
    document.set_main(key, Some(value));
}

fn set_optional(document: &mut DesktopDocument, key: &str, value: &str) {
    document.set_main(key, (!value.is_empty()).then_some(value));
}

fn set_localizable_optional(
    document: &mut DesktopDocument,
    key: &str,
    value: &str,
    localized_values: &[LocalizedValue],
) {
    let has_localized_value = localized_values
        .iter()
        .any(|localized| localized.key == key);
    document.set_main(
        key,
        (!value.is_empty() || has_localized_value).then_some(value),
    );
}

fn set_localizable_list(
    document: &mut DesktopDocument,
    key: &str,
    value: &str,
    localized_values: &[LocalizedValue],
) {
    let has_localized_value = localized_values
        .iter()
        .any(|localized| localized.key == key);
    document.set_main(
        key,
        (!value.trim().is_empty() || has_localized_value).then_some(value),
    );
}

fn set_list(document: &mut DesktopDocument, key: &str, value: &str) {
    document.set_main(key, (!value.trim().is_empty()).then_some(value));
}

fn apply_optional_boolean(dropdown: &gtk::DropDown, value: Option<&str>) {
    let selected = match value {
        None => 0,
        Some("true") => 1,
        Some("false") => 2,
        Some(_) => gtk::INVALID_LIST_POSITION,
    };
    dropdown.set_selected(selected);
}

fn set_optional_boolean(document: &mut DesktopDocument, key: &str, dropdown: &gtk::DropDown) {
    match dropdown.selected() {
        0 => document.set_main(key, None),
        1 => document.set_main(key, Some("true")),
        2 => document.set_main(key, Some("false")),
        _ => {}
    }
}

fn connect_icon_preview(widgets: &EditorWidgets) {
    let preview = widgets.icon_preview.clone();
    widgets.icon.connect_changed(move |entry| {
        update_icon_preview(&preview, entry.text().as_str());
    });
}

fn update_icon_preview(preview: &gtk::Image, value: &str) {
    let value = value.trim();
    if value.is_empty() {
        preview.clear();
        preview.set_tooltip_text(None);
        return;
    }
    if Path::new(value).is_absolute() {
        preview.set_icon_name(Some("image-x-generic-symbolic"));
        preview.set_tooltip_text(Some("Absolute icon paths are not decoded by DeskEntry"));
        return;
    }

    let theme = gtk::IconTheme::for_display(&preview.display());
    if theme.has_icon(value) {
        preview.set_icon_name(Some(value));
        preview.set_tooltip_text(None);
    } else {
        preview.set_icon_name(Some("image-missing-symbolic"));
        preview.set_tooltip_text(Some("Icon name was not found in the active icon theme"));
    }
}

fn connect_locale_add(widgets: &EditorWidgets) {
    let widgets = widgets.clone();
    let sink = widgets.dirty_sink.clone();
    widgets.add_locale.clone().connect_clicked(move |_| {
        mark_dirty_sink(&sink);
        add_locale_row(&widgets, None);
    });
}

fn add_locale_row(widgets: &EditorWidgets, initial: Option<LocalizedValue>) {
    let row = gtk::ListBoxRow::new();
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    content.set_margin_top(6);
    content.set_margin_bottom(6);
    content.set_margin_start(8);
    content.set_margin_end(8);

    let key = gtk::DropDown::from_strings(&LOCALIZED_KEYS);
    let selected_key = initial.as_ref().map_or("Name", |value| value.key.as_str());
    let selected = LOCALIZED_KEYS
        .iter()
        .position(|value| *value == selected_key)
        .and_then(|index| u32::try_from(index).ok())
        .unwrap_or_default();
    key.set_selected(selected);

    let locale = gtk::Entry::new();
    locale.set_placeholder_text(Some("pl_PL"));
    locale.set_width_chars(12);
    let value = gtk::Entry::new();
    value.set_hexpand(true);
    value.set_placeholder_text(Some("Localized value"));
    let remove = gtk::Button::from_icon_name("user-trash-symbolic");
    remove.set_tooltip_text(Some("Remove translation"));

    if let Some(initial) = initial {
        locale.set_text(&initial.locale);
        value.set_text(&initial.value);
    }

    content.append(&key);
    content.append(&locale);
    content.append(&value);
    content.append(&remove);
    row.set_child(Some(&content));
    widgets.locale_list.append(&row);
    connect_dropdown_dirty(widgets, &key);
    connect_entry_dirty(widgets, &locale);
    connect_entry_dirty(widgets, &value);
    widgets.locale_rows.borrow_mut().push(LocaleRow {
        row: row.clone(),
        key,
        locale,
        value,
    });

    let list = widgets.locale_list.clone();
    let rows = widgets.locale_rows.clone();
    let sink = widgets.dirty_sink.clone();
    remove.connect_clicked(move |_| {
        list.remove(&row);
        rows.borrow_mut().retain(|item| item.row != row);
        mark_dirty_sink(&sink);
    });
}

fn clear_locale_rows(widgets: &EditorWidgets) {
    while let Some(child) = widgets.locale_list.first_child() {
        widgets.locale_list.remove(&child);
    }
    widgets.locale_rows.borrow_mut().clear();
}

fn collect_locale_values(widgets: &EditorWidgets) -> Vec<LocalizedValue> {
    widgets
        .locale_rows
        .borrow()
        .iter()
        .filter_map(|row| {
            let index = usize::try_from(row.key.selected()).ok()?;
            let key = LOCALIZED_KEYS.get(index)?.to_string();
            let locale = row.locale.text().trim().to_string();
            let value = row.value.text().to_string();
            if locale.is_empty() {
                return None;
            }
            Some(LocalizedValue { key, locale, value })
        })
        .collect()
}

fn connect_action_add(widgets: &EditorWidgets) {
    let widgets = widgets.clone();
    let sink = widgets.dirty_sink.clone();
    widgets.add_action.clone().connect_clicked(move |_| {
        mark_dirty_sink(&sink);
        add_action_row(&widgets, None);
    });
}

fn add_action_row(widgets: &EditorWidgets, initial: Option<DesktopAction>) {
    let row = gtk::ListBoxRow::new();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
    content.set_margin_top(10);
    content.set_margin_bottom(10);
    content.set_margin_start(10);
    content.set_margin_end(10);

    let fields = build_action_fields(widgets, initial.as_ref());
    content.append(&fields.grid);

    let locale_list = gtk::ListBox::new();
    locale_list.set_selection_mode(gtk::SelectionMode::None);
    locale_list.add_css_class("boxed-list");
    let add_locale = gtk::Button::with_label("Add action translation");
    add_locale.set_halign(gtk::Align::Start);
    content.append(&form_label("Localized fields"));
    content.append(&locale_list);
    content.append(&add_locale);

    row.set_child(Some(&content));
    widgets.action_list.append(&row);

    let localized_rows = Rc::new(RefCell::new(Vec::new()));
    if let Some(action) = initial.as_ref() {
        for localized in &action.localized_values {
            add_action_locale_row(
                &locale_list,
                &localized_rows,
                &widgets.dirty_sink,
                Some(localized),
            );
        }
    }

    connect_entry_dirty(widgets, &fields.id);
    connect_entry_dirty(widgets, &fields.name);
    connect_entry_dirty(widgets, &fields.exec);
    connect_entry_dirty(widgets, &fields.icon);

    let remove = fields.remove.clone();
    let action_row = DesktopActionRow {
        row: row.clone(),
        original_id: initial.and_then(|action| action.original_id),
        id: fields.id,
        name: fields.name,
        exec: fields.exec,
        icon: fields.icon,
        localized_rows: localized_rows.clone(),
    };
    widgets.action_rows.borrow_mut().push(action_row);

    let rows = widgets.action_rows.clone();
    let list = widgets.action_list.clone();
    let remove_sink = widgets.dirty_sink.clone();
    remove.connect_clicked(move |_| {
        list.remove(&row);
        rows.borrow_mut().retain(|item| item.row != row);
        mark_dirty_sink(&remove_sink);
    });

    let dirty_sink = widgets.dirty_sink.clone();
    let add_sink = dirty_sink.clone();
    add_locale.connect_clicked(move |_| {
        mark_dirty_sink(&add_sink);
        add_action_locale_row(&locale_list, &localized_rows, &dirty_sink, None);
    });
}

fn build_action_fields(
    widgets: &EditorWidgets,
    initial: Option<&DesktopAction>,
) -> ActionFieldWidgets {
    let grid = gtk::Grid::new();
    grid.set_row_spacing(8);
    grid.set_column_spacing(10);

    let id = add_entry_row(&grid, 0, "Action ID");
    let name = add_entry_row(&grid, 1, "Name");
    let exec = add_entry_row(&grid, 2, "Exec");
    let icon = add_entry_row(&grid, 3, "Icon");
    let remove = gtk::Button::with_label("Remove action");
    remove.add_css_class("destructive-action");
    grid.attach(&remove, 1, 4, 1, 1);

    if let Some(action) = initial {
        id.set_text(&action.id);
        name.set_text(&action.name);
        exec.set_text(&action.exec);
        icon.set_text(&action.icon);
    } else {
        if let Some(next_id) = next_action_id(widgets) {
            id.set_text(&next_id);
        } else {
            set_status(widgets, "Could not allocate a Desktop Action ID");
        }
        name.set_text("New Action");
    }

    ActionFieldWidgets {
        grid,
        id,
        name,
        exec,
        icon,
        remove,
    }
}

fn next_action_id(widgets: &EditorWidgets) -> Option<String> {
    let rows = widgets.action_rows.borrow();
    let candidate_count = rows.len().checked_add(1)?;

    (1..=candidate_count)
        .map(|index| {
            if index == 1 {
                "NewAction".to_string()
            } else {
                format!("NewAction{index}")
            }
        })
        .find(|candidate| {
            rows.iter()
                .all(|row| row.id.text().as_str() != candidate.as_str())
        })
}

fn add_action_locale_row(
    list: &gtk::ListBox,
    rows: &Rc<RefCell<Vec<ActionLocaleRow>>>,
    dirty_sink: &Rc<RefCell<Option<DirtySink>>>,
    initial: Option<&ActionLocalizedValue>,
) {
    let row = gtk::ListBoxRow::new();
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    content.set_margin_top(6);
    content.set_margin_bottom(6);
    content.set_margin_start(8);
    content.set_margin_end(8);

    let key = gtk::DropDown::from_strings(&ACTION_LOCALIZED_KEYS);
    let selected_key = initial.map_or("Name", |value| value.key.as_str());
    let selected = ACTION_LOCALIZED_KEYS
        .iter()
        .position(|value| *value == selected_key)
        .and_then(|index| u32::try_from(index).ok())
        .unwrap_or_default();
    key.set_selected(selected);

    let locale = gtk::Entry::new();
    locale.set_placeholder_text(Some("pl_PL"));
    locale.set_width_chars(12);
    let value = gtk::Entry::new();
    value.set_placeholder_text(Some("Localized action value"));
    value.set_hexpand(true);
    let remove = gtk::Button::from_icon_name("user-trash-symbolic");
    remove.set_tooltip_text(Some("Remove action translation"));

    if let Some(initial) = initial {
        locale.set_text(&initial.locale);
        value.set_text(&initial.value);
    }

    content.append(&key);
    content.append(&locale);
    content.append(&value);
    content.append(&remove);
    row.set_child(Some(&content));
    list.append(&row);
    connect_dropdown_dirty_sink(dirty_sink, &key);
    connect_entry_dirty_sink(dirty_sink, &locale);
    connect_entry_dirty_sink(dirty_sink, &value);
    rows.borrow_mut().push(ActionLocaleRow {
        row: row.clone(),
        key,
        locale,
        value,
    });

    let list = list.clone();
    let rows = rows.clone();
    let dirty_sink = dirty_sink.clone();
    remove.connect_clicked(move |_| {
        list.remove(&row);
        rows.borrow_mut().retain(|item| item.row != row);
        mark_dirty_sink(&dirty_sink);
    });
}

fn clear_action_rows(widgets: &EditorWidgets) {
    while let Some(child) = widgets.action_list.first_child() {
        widgets.action_list.remove(&child);
    }
    widgets.action_rows.borrow_mut().clear();
}

fn collect_desktop_actions(widgets: &EditorWidgets) -> Vec<DesktopAction> {
    widgets
        .action_rows
        .borrow()
        .iter()
        .map(|row| DesktopAction {
            original_id: row.original_id.clone(),
            id: row.id.text().to_string(),
            name: row.name.text().to_string(),
            exec: row.exec.text().to_string(),
            icon: row.icon.text().to_string(),
            localized_values: collect_action_localized_values(row),
        })
        .collect()
}

fn collect_action_localized_values(row: &DesktopActionRow) -> Vec<ActionLocalizedValue> {
    row.localized_rows
        .borrow()
        .iter()
        .filter_map(|localized| {
            let index = usize::try_from(localized.key.selected()).ok()?;
            let key = ACTION_LOCALIZED_KEYS.get(index)?.to_string();
            let locale = localized.locale.text().trim().to_string();
            if locale.is_empty() {
                return None;
            }
            Some(ActionLocalizedValue {
                key,
                locale,
                value: localized.value.text().to_string(),
            })
        })
        .collect()
}

fn connect_extra_add(widgets: &EditorWidgets) {
    let editor = widgets.clone();
    widgets.add_extra.clone().connect_clicked(move |_| {
        add_extra_row(&editor, None);
        mark_dirty_sink(&editor.dirty_sink);
    });
}

fn add_extra_row(widgets: &EditorWidgets, initial: Option<ExtraKeyValue>) {
    let row = gtk::ListBoxRow::new();
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    content.set_margin_top(6);
    content.set_margin_bottom(6);
    content.set_margin_start(8);
    content.set_margin_end(8);

    let group = gtk::Entry::new();
    group.set_placeholder_text(Some("Desktop Entry"));
    group.set_width_chars(18);
    let key = gtk::Entry::new();
    key.set_placeholder_text(Some("X-Vendor-Key"));
    key.set_width_chars(18);
    let value = gtk::Entry::new();
    value.set_placeholder_text(Some("Value"));
    value.set_hexpand(true);
    let remove = gtk::Button::from_icon_name("user-trash-symbolic");
    remove.set_tooltip_text(Some("Remove extra key"));

    if let Some(initial) = initial {
        group.set_text(&initial.group);
        key.set_text(&initial.key);
        value.set_text(&initial.value);
    } else {
        group.set_text("Desktop Entry");
    }

    content.append(&group);
    content.append(&key);
    content.append(&value);
    content.append(&remove);
    row.set_child(Some(&content));
    widgets.extra_list.append(&row);

    connect_entry_dirty_sink(&widgets.dirty_sink, &group);
    connect_entry_dirty_sink(&widgets.dirty_sink, &key);
    connect_entry_dirty_sink(&widgets.dirty_sink, &value);
    widgets.extra_rows.borrow_mut().push(ExtraKeyRow {
        row: row.clone(),
        group,
        key,
        value,
    });

    let list = widgets.extra_list.clone();
    let rows = widgets.extra_rows.clone();
    let dirty_sink = widgets.dirty_sink.clone();
    remove.connect_clicked(move |_| {
        list.remove(&row);
        rows.borrow_mut().retain(|item| item.row != row);
        mark_dirty_sink(&dirty_sink);
    });
}

fn clear_extra_rows(widgets: &EditorWidgets) {
    while let Some(child) = widgets.extra_list.first_child() {
        widgets.extra_list.remove(&child);
    }
    widgets.extra_rows.borrow_mut().clear();
}

fn collect_extra_values(widgets: &EditorWidgets) -> Vec<ExtraKeyValue> {
    widgets
        .extra_rows
        .borrow()
        .iter()
        .filter_map(|row| {
            let group = row.group.text().trim().to_string();
            let key = row.key.text().trim().to_string();
            if group.is_empty() && key.is_empty() && row.value.text().is_empty() {
                return None;
            }
            Some(ExtraKeyValue {
                group,
                key,
                value: row.value.text().to_string(),
            })
        })
        .collect()
}

fn connect_type_awareness(widgets: &EditorWidgets) {
    let entry_type = widgets.entry_type.clone();
    let editor = widgets.clone();
    entry_type.connect_changed(move |_| {
        update_type_aware_fields(&editor);
    });
}

fn update_type_aware_fields(widgets: &EditorWidgets) {
    let entry_type = widgets.entry_type.text();
    let is_application = entry_type.as_str() == "Application";
    let is_link = entry_type.as_str() == "Link";
    let is_directory = entry_type.as_str() == "Directory";
    let is_known = is_application || is_link || is_directory;
    let application_fields = is_application || !is_known;
    let link_fields = is_link || !is_known;

    widgets.exec.set_sensitive(application_fields);
    widgets.categories.set_sensitive(application_fields);
    widgets.keywords.set_sensitive(application_fields);
    widgets.mime_type.set_sensitive(application_fields);
    widgets.try_exec.set_sensitive(application_fields);
    widgets.path.set_sensitive(application_fields);
    widgets.terminal.set_sensitive(application_fields);
    widgets.startup_notify.set_sensitive(application_fields);
    widgets.dbus_activatable.set_sensitive(application_fields);
    widgets
        .prefers_non_default_gpu
        .set_sensitive(application_fields);
    widgets.single_main_window.set_sensitive(application_fields);
    widgets.action_list.set_sensitive(application_fields);
    widgets.add_action.set_sensitive(application_fields);
    widgets.url.set_sensitive(link_fields);
}

fn connect_page_refresh(state: &Rc<RefCell<AppState>>, widgets: &AppWidgets) {
    let state = state.clone();
    let widgets = widgets.clone();
    widgets
        .editor
        .page_stack
        .clone()
        .connect_visible_child_name_notify(move |stack| {
            let Some(name) = stack.visible_child_name() else {
                return;
            };
            match name.as_str() {
                "raw" => refresh_raw_preview(&state, &widgets.editor),
                "source" => refresh_source_comparison(&state, &widgets),
                _ => {}
            }
        });
}

fn current_editor_document(
    state: &Rc<RefCell<AppState>>,
    widgets: &EditorWidgets,
) -> Result<DesktopDocument, String> {
    let mut document = state
        .borrow()
        .document
        .clone()
        .ok_or_else(|| "Select or create a desktop entry first".to_string())?;
    apply_widgets_to_document(widgets, &mut document)?;
    Ok(document)
}

fn refresh_raw_preview(state: &Rc<RefCell<AppState>>, widgets: &EditorWidgets) {
    let Ok(document) = current_editor_document(state, widgets) else {
        return;
    };
    widgets.raw_buffer.set_text(&document.serialize());
}

fn original_variant(state: &Rc<RefCell<AppState>>) -> Option<scanner::EntryVariant> {
    let borrowed = state.borrow();
    let id = borrowed.selected_id.as_ref()?;
    let group = borrowed.groups.iter().find(|group| &group.id == id)?;
    let effective = group.effective()?;
    if effective.source != SourceKind::User {
        return None;
    }
    group.variants.get(1).cloned()
}

fn refresh_source_comparison(state: &Rc<RefCell<AppState>>, widgets: &AppWidgets) {
    clear_list(&widgets.editor.compare_list);
    widgets.editor.original_path.set_text("");
    widgets.editor.view_original.set_sensitive(false);

    let Some(original) = original_variant(state) else {
        widgets
            .editor
            .compare_status
            .set_text("No lower-priority original is available for this entry.");
        return;
    };

    widgets
        .editor
        .original_path
        .set_text(&original.path.to_string_lossy());
    widgets.editor.view_original.set_sensitive(true);

    let current = match current_editor_document(state, &widgets.editor) {
        Ok(document) => document,
        Err(error) => {
            widgets.editor.compare_status.set_text(&format!(
                "Fix the current editor values before comparing: {error}"
            ));
            return;
        }
    };
    let differences = comparison::compare_documents(&current, &original.document);
    if differences.is_empty() {
        widgets
            .editor
            .compare_status
            .set_text("The user override matches the current original.");
        return;
    }

    widgets.editor.compare_status.set_text(&format!(
        "{} differing key(s). Use Original changes only the selected key.",
        differences.len()
    ));
    for difference in differences {
        append_comparison_row(state, widgets, difference);
    }
}

fn append_comparison_row(
    state: &Rc<RefCell<AppState>>,
    widgets: &AppWidgets,
    difference: KeyDifference,
) {
    let row = gtk::ListBoxRow::new();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
    content.set_margin_top(8);
    content.set_margin_bottom(8);
    content.set_margin_start(10);
    content.set_margin_end(10);

    let title = gtk::Label::new(Some(&format!(
        "[{}] {}",
        difference.key.group, difference.key.key
    )));
    title.set_xalign(0.0);
    title.add_css_class("heading");

    let values = gtk::Grid::new();
    values.set_column_spacing(12);
    values.set_row_spacing(6);
    let current_label = source_label();
    current_label.set_selectable(true);
    current_label.set_text(&comparison::display_values(&difference.override_values));
    let original_label = source_label();
    original_label.set_selectable(true);
    original_label.set_text(&comparison::display_values(&difference.original_values));
    values.attach(&form_label("Override"), 0, 0, 1, 1);
    values.attach(&current_label, 1, 0, 1, 1);
    values.attach(&form_label("Original"), 0, 1, 1, 1);
    values.attach(&original_label, 1, 1, 1, 1);

    let use_original = gtk::Button::with_label("Use Original");
    use_original.set_halign(gtk::Align::Start);

    content.append(&title);
    content.append(&values);
    content.append(&use_original);
    row.set_child(Some(&content));
    widgets.editor.compare_list.append(&row);

    let state = state.clone();
    let widgets = widgets.clone();
    use_original.connect_clicked(move |_| {
        let mut document = match current_editor_document(&state, &widgets.editor) {
            Ok(document) => document,
            Err(error) => {
                set_status(&widgets.editor, &error);
                return;
            }
        };
        if let Err(error) = comparison::apply_original_value(&mut document, &difference) {
            set_status(&widgets.editor, &error);
            return;
        }
        apply_document(&widgets.editor, &document);
        state.borrow_mut().document = Some(document);
        mark_dirty_sink(&widgets.editor.dirty_sink);
        refresh_raw_preview(&state, &widgets.editor);
        refresh_source_comparison(&state, &widgets);
        set_status(&widgets.editor, "Applied the original value to the editor");
    });
}

fn open_desktop_file_dialog(state: &Rc<RefCell<AppState>>, widgets: &AppWidgets) {
    let dialog = gtk::FileDialog::builder()
        .title("Open Desktop File")
        .modal(true)
        .build();
    let state = state.clone();
    let widgets = widgets.clone();
    let parent = widgets.window.clone();
    dialog.open(Some(&parent), None::<&gio::Cancellable>, move |result| {
        let Ok(file) = result else {
            return;
        };
        let Some(path) = file.path() else {
            set_status(
                &widgets.editor,
                "Selected desktop file has no local filesystem path",
            );
            return;
        };
        match storage::load_desktop_file(&path) {
            Ok(loaded) => load_external_desktop_file(&state, &widgets, loaded),
            Err(error) => set_status(&widgets.editor, &error),
        }
    });
}

fn load_external_desktop_file(
    state: &Rc<RefCell<AppState>>,
    widgets: &AppWidgets,
    loaded: storage::LoadedDesktopFile,
) {
    let storage::LoadedDesktopFile {
        path,
        suggested_id,
        document,
    } = loaded;

    widgets.list.unselect_all();
    apply_document(&widgets.editor, &document);
    {
        let mut borrowed = state.borrow_mut();
        borrowed.selected_id = None;
        borrowed.document = Some(document);
        borrowed.effective_path = Some(path.clone());
        borrowed.baseline = None;
        borrowed.persistence = EntryPersistence::Unsaved;
    }
    set_editor_sensitive(&widgets.editor, true);
    widgets.editor.id.set_text(&suggested_id);
    widgets.editor.id.set_sensitive(true);
    widgets.editor.source.set_text("External file");
    widgets.editor.file_path.set_text(&path.to_string_lossy());
    widgets
        .editor
        .override_info
        .set_text("Saving imports this file into the XDG user applications directory");
    widgets.editor.original_path.set_text("");
    widgets.editor.view_original.set_sensitive(false);
    widgets
        .editor
        .compare_status
        .set_text("External files have no XDG original comparison.");
    clear_list(&widgets.editor.compare_list);
    widgets.editor.reset_override.set_sensitive(false);
    widgets.editor.delete_entry.set_sensitive(false);
    widgets.editor.open_location.set_sensitive(true);
    widgets.editor.launch.set_sensitive(false);
    set_clean_baseline(state, &widgets.editor);
    refresh_raw_preview(state, &widgets.editor);
    set_status(&widgets.editor, "Opened external desktop file");
}

fn connect_icon_browser(widgets: &AppWidgets) {
    let widgets = widgets.clone();
    widgets
        .editor
        .browse_icon
        .clone()
        .connect_clicked(move |_| {
            show_icon_browser(&widgets);
        });
}

fn icon_browser_factory() -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, object| {
        let Some(item) = object.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        content.set_margin_top(6);
        content.set_margin_bottom(6);
        content.set_margin_start(10);
        content.set_margin_end(10);
        let image = gtk::Image::new();
        image.set_pixel_size(32);
        let label = gtk::Label::new(None);
        label.set_xalign(0.0);
        label.set_hexpand(true);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        content.append(&image);
        content.append(&label);
        item.set_child(Some(&content));
    });
    factory.connect_bind(|_, object| {
        let Some(item) = object.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(name) = item.item().and_downcast::<gtk::StringObject>() else {
            return;
        };
        let Some(content) = item.child().and_downcast::<gtk::Box>() else {
            return;
        };
        let name = name.string();
        if let Some(image) = content.first_child().and_downcast::<gtk::Image>() {
            image.set_icon_name(Some(name.as_str()));
        }
        if let Some(label) = content.last_child().and_downcast::<gtk::Label>() {
            label.set_label(name.as_str());
            label.set_tooltip_text(Some(name.as_str()));
        }
    });
    factory
}

fn update_icon_browser_status(status: &gtk::Label, matches: u32) {
    if matches == 0 {
        status.set_label("No icons found");
    } else {
        status.set_label(&format!("{matches} icons"));
    }
}

fn icon_browser_footer(
    window: &adw::Window,
    selection: &gtk::SingleSelection,
    icon_entry: &gtk::Entry,
) -> gtk::Box {
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    footer.set_margin_start(12);
    footer.set_margin_end(12);
    footer.set_margin_bottom(12);
    let hint = gtk::Label::new(Some("Double-click an icon or select it and click Choose."));
    hint.set_wrap(true);
    hint.set_xalign(0.0);
    hint.set_hexpand(true);
    hint.add_css_class("dim-label");
    let cancel = gtk::Button::with_label("Cancel");
    let choose = gtk::Button::with_label("Choose");
    choose.add_css_class("suggested-action");
    choose.set_sensitive(selection.selected_item().is_some());
    footer.append(&hint);
    footer.append(&cancel);
    footer.append(&choose);
    let choose_button = choose.downgrade();
    selection.connect_selected_item_notify(move |selection| {
        if let Some(choose_button) = choose_button.upgrade() {
            choose_button.set_sensitive(selection.selected_item().is_some());
        }
    });
    let browser = window.downgrade();
    cancel.connect_clicked(move |_| {
        if let Some(browser) = browser.upgrade() {
            browser.close();
        }
    });
    let icon_entry = icon_entry.clone();
    let browser = window.downgrade();
    let selected_icon = selection.clone();
    choose.connect_clicked(move |_| {
        if let Some(name) = selected_icon
            .selected_item()
            .and_downcast::<gtk::StringObject>()
            && let Some(browser) = browser.upgrade()
        {
            icon_entry.set_text(name.string().as_str());
            browser.close();
        }
    });
    footer
}

fn load_icon_browser_names(
    window: &adw::Window,
    names: gtk::StringList,
    spinner: gtk::Spinner,
    status: gtk::Label,
    filtered: gtk::FilterListModel,
) {
    let browser = window.downgrade();
    gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(80), move || {
        let Some(browser) = browser
            .upgrade()
            .filter(gtk::prelude::WidgetExt::is_visible)
        else {
            return;
        };
        let display = gtk::prelude::WidgetExt::display(&browser);
        let theme = gtk::IconTheme::for_display(&display);
        let mut icon_names = theme
            .icon_names()
            .into_iter()
            .map(|name| name.to_string())
            .collect::<Vec<_>>();
        icon_names.sort_unstable();
        icon_names.dedup();
        let values = icon_names.iter().map(String::as_str).collect::<Vec<_>>();
        names.splice(0, 0, &values);
        spinner.stop();
        spinner.set_visible(false);
        update_icon_browser_status(&status, filtered.n_items());
    });
}

fn show_icon_browser(widgets: &AppWidgets) {
    let window = adw::Window::builder()
        .title("Choose Theme Icon")
        .default_width(620)
        .default_height(680)
        .modal(true)
        .transient_for(&widgets.window)
        .build();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let header = adw::HeaderBar::new();
    root.append(&header);
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("Search theme icons"));
    search.set_margin_start(12);
    search.set_margin_end(12);
    root.append(&search);

    let status_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    status_row.set_margin_start(12);
    status_row.set_margin_end(12);
    let spinner = gtk::Spinner::new();
    spinner.start();
    let status = gtk::Label::new(Some("Loading theme icons…"));
    status.add_css_class("dim-label");
    status_row.append(&spinner);
    status_row.append(&status);
    root.append(&status_row);

    let names = gtk::StringList::new(&[]);
    let expression = gtk::PropertyExpression::new(
        gtk::StringObject::static_type(),
        None::<gtk::Expression>,
        "string",
    );
    let filter = gtk::StringFilter::new(Some(expression));
    filter.set_match_mode(gtk::StringFilterMatchMode::Substring);
    filter.set_ignore_case(true);
    let filtered = gtk::FilterListModel::new(Some(names.clone()), Some(filter.clone()));
    let selection = gtk::SingleSelection::new(Some(filtered.clone()));
    selection.set_autoselect(false);
    selection.set_can_unselect(true);
    let list = gtk::ListView::new(Some(selection.clone()), Some(icon_browser_factory()));
    list.set_single_click_activate(false);
    list.add_css_class("boxed-list");

    let scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&list)
        .build();
    root.append(&scroll);

    root.append(&icon_browser_footer(
        &window,
        &selection,
        &widgets.editor.icon,
    ));
    window.set_content(Some(&root));

    search.connect_search_changed(move |entry| {
        let text = entry.text();
        let query = text.trim();
        filter.set_search((!query.is_empty()).then_some(query));
        selection.set_selected(gtk::INVALID_LIST_POSITION);
    });

    let result_status = status.clone();
    let loading_spinner = spinner.clone();
    filtered.connect_items_changed(move |model, _, _, _| {
        if !loading_spinner.is_spinning() {
            update_icon_browser_status(&result_status, model.n_items());
        }
    });

    let icon_entry = widgets.editor.icon.clone();
    let browser = window.downgrade();
    let activate_model = filtered.clone();
    list.connect_activate(move |_, position| {
        if let Some(name) = activate_model
            .item(position)
            .and_downcast::<gtk::StringObject>()
            && let Some(browser) = browser.upgrade()
        {
            icon_entry.set_text(name.string().as_str());
            browser.close();
        }
    });

    window.present();
    search.grab_focus();
    load_icon_browser_names(&window, names, spinner, status, filtered);
}

fn set_editor_sensitive(widgets: &EditorWidgets, sensitive: bool) {
    widgets
        .view_stack
        .set_visible_child_name(if sensitive { "editor" } else { "empty" });
    widgets.name.set_sensitive(sensitive);
    widgets.generic_name.set_sensitive(sensitive);
    widgets.comment.set_sensitive(sensitive);
    widgets.exec.set_sensitive(sensitive);
    widgets.icon.set_sensitive(sensitive);
    widgets.import_icon.set_sensitive(sensitive);
    widgets.browse_icon.set_sensitive(sensitive);
    widgets.categories.set_sensitive(sensitive);
    widgets.keywords.set_sensitive(sensitive);
    widgets.mime_type.set_sensitive(sensitive);
    widgets.entry_type.set_sensitive(sensitive);
    widgets.version.set_sensitive(sensitive);
    widgets.try_exec.set_sensitive(sensitive);
    widgets.path.set_sensitive(sensitive);
    widgets.url.set_sensitive(sensitive);
    widgets.only_show_in.set_sensitive(sensitive);
    widgets.not_show_in.set_sensitive(sensitive);
    widgets.implements.set_sensitive(sensitive);
    widgets.action_list.set_sensitive(sensitive);
    widgets.add_action.set_sensitive(sensitive);
    widgets.extra_list.set_sensitive(sensitive);
    widgets.add_extra.set_sensitive(sensitive);
    widgets.terminal.set_sensitive(sensitive);
    widgets.no_display.set_sensitive(sensitive);
    widgets.hidden.set_sensitive(sensitive);
    widgets.startup_notify.set_sensitive(sensitive);
    widgets.dbus_activatable.set_sensitive(sensitive);
    widgets.prefers_non_default_gpu.set_sensitive(sensitive);
    widgets.single_main_window.set_sensitive(sensitive);
    widgets.locale_list.set_sensitive(sensitive);
    widgets.add_locale.set_sensitive(sensitive);
    widgets.validate.set_sensitive(sensitive);
    widgets.revert.set_sensitive(sensitive);
    widgets.open_location.set_sensitive(false);
    widgets.launch.set_sensitive(false);
    widgets.reset_override.set_sensitive(false);
    widgets.delete_entry.set_sensitive(false);
    widgets.view_original.set_sensitive(false);
    if sensitive {
        update_type_aware_fields(widgets);
    } else {
        widgets.id.set_sensitive(false);
        clear_dirty_indicator(widgets);
    }
}

fn add_boolean_row(grid: &gtk::Grid, row: i32, label: &str) -> gtk::DropDown {
    let dropdown = gtk::DropDown::from_strings(&BOOLEAN_STATES);
    dropdown.set_selected(0);
    grid.attach(&form_label(label), 0, row, 1, 1);
    grid.attach(&dropdown, 1, row, 1, 1);
    dropdown
}

fn form_grid() -> gtk::Grid {
    let grid = gtk::Grid::new();
    grid.set_row_spacing(10);
    grid.set_column_spacing(12);
    grid.set_margin_top(12);
    grid.set_margin_bottom(12);
    grid.set_margin_start(12);
    grid.set_margin_end(12);
    grid.set_hexpand(true);
    grid
}

fn form_label(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(1.0);
    label.set_valign(gtk::Align::Center);
    label
}

fn source_label() -> gtk::Label {
    let label = gtk::Label::new(None);
    label.set_xalign(0.0);
    label.set_wrap(true);
    label
}

fn add_entry_row(grid: &gtk::Grid, row: i32, label: &str) -> gtk::Entry {
    let entry = gtk::Entry::new();
    entry.set_hexpand(true);
    grid.attach(&form_label(label), 0, row, 1, 1);
    grid.attach(&entry, 1, row, 1, 1);
    entry
}

fn scrolled(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .child(child)
        .build()
}

fn set_success_status(widgets: &EditorWidgets, success: &str, warning: Option<&str>) {
    let message = warning.map_or_else(
        || success.to_string(),
        |warning| format!("{success}. Warning: {warning}"),
    );
    set_status(widgets, &message);
}

fn set_status(widgets: &EditorWidgets, message: &str) {
    widgets.status.set_text(message);
    widgets.status.set_tooltip_text(Some(message));
}

#[cfg(test)]
mod tests {
    use super::{
        AppState, EntryPersistence, SelectionSignal, revert_selected_entry, source_matches_filter,
        take_suppressed_selection, type_matches_filter,
    };
    use crate::scanner::SourceKind;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn revert_releases_selection_borrow_before_loading() {
        let state = RefCell::new(AppState {
            selected_id: Some("example.desktop".to_string()),
            ..AppState::default()
        });
        assert!(revert_selected_entry(&state, |id| {
            assert_eq!(id, "example.desktop");
            state.borrow_mut().selected_id = None;
        }));
        assert!(!revert_selected_entry(&state, |_| {}));
    }

    #[test]
    fn source_filters_group_flatpak_variants() {
        assert!(source_matches_filter(3, SourceKind::FlatpakUser));
        assert!(source_matches_filter(3, SourceKind::FlatpakSystem));
        assert!(!source_matches_filter(3, SourceKind::System));
    }

    #[test]
    fn source_filters_keep_user_and_system_separate() {
        assert!(source_matches_filter(1, SourceKind::User));
        assert!(!source_matches_filter(1, SourceKind::System));
        assert!(source_matches_filter(2, SourceKind::System));
        assert!(!source_matches_filter(2, SourceKind::User));
    }

    #[test]
    fn type_filters_match_standard_types() {
        assert!(type_matches_filter(1, "Application"));
        assert!(type_matches_filter(2, "Link"));
        assert!(type_matches_filter(3, "Directory"));
        assert!(!type_matches_filter(1, "Link"));
    }

    #[test]
    fn type_filter_other_keeps_unknown_types() {
        assert!(type_matches_filter(4, "Service"));
        assert!(type_matches_filter(4, ""));
        assert!(!type_matches_filter(4, "Application"));
    }

    #[test]
    fn unsaved_persistence_requires_initial_save() {
        assert!(!EntryPersistence::Existing.needs_initial_save());
        assert!(EntryPersistence::Unsaved.needs_initial_save());
    }

    #[test]
    fn suppressed_selection_is_consumed_once() {
        let state = Rc::new(RefCell::new(AppState::default()));
        state.borrow_mut().selection_signal = SelectionSignal::SuppressNext;

        assert!(take_suppressed_selection(&state));
        assert!(!take_suppressed_selection(&state));
    }
}
