// EffectCraft scripting: ScriptUI, the dialog / panel object model After Effects scripts use
// (`new Window("dialog", "Title")`, `win.add("button", undefined, "OK")`, `onClick`, `show()`…).
//
// Written from the behaviour Adobe's public JavaScript Tools Guide documents. Controls are plain
// JavaScript objects; after each script step the host reads `__uiSnapshot()` (a JSON
// description of the open windows) and lays them out, frontends draw them, and user actions come
// back through `__uiDispatch(window, control, kind, valueJson)`. Modal dialogs block in
// `show()` (`__uiModal`) until they close, as in After Effects.

var __uiWins = [];
// show() results of closed dialogs, by window id.
var __uiResults = {};

function __uiModalResult(id) {
  var r = __uiResults[id];
  if (r !== undefined) {
    delete __uiResults[id];
    return r;
  }
  return __uiWin(id) ? null : 2;
}

function __dim(w, h) {
  var d = [w, h];
  Object.defineProperty(d, "width", { get: function () { return this[0]; }, set: function (v) { this[0] = v; } });
  Object.defineProperty(d, "height", { get: function () { return this[1]; }, set: function (v) { this[1] = v; } });
  return d;
}
function __bounds4(l, t, r, b) {
  var a = [l, t, r, b];
  Object.defineProperty(a, "left", { get: function () { return this[0]; } });
  Object.defineProperty(a, "top", { get: function () { return this[1]; } });
  Object.defineProperty(a, "right", { get: function () { return this[2]; } });
  Object.defineProperty(a, "bottom", { get: function () { return this[3]; } });
  Object.defineProperty(a, "x", { get: function () { return this[0]; } });
  Object.defineProperty(a, "y", { get: function () { return this[1]; } });
  Object.defineProperty(a, "width", { get: function () { return this[2] - this[0]; } });
  Object.defineProperty(a, "height", { get: function () { return this[3] - this[1]; } });
  return a;
}
function __toBounds(b) {
  if (b === undefined || b === null) return null;
  if (b.length === 4) return [__num(b[0]), __num(b[1]), __num(b[2]), __num(b[3])];
  if (b.width !== undefined) {
    var x = b.x || b.left || 0, y = b.y || b.top || 0;
    return [x, y, x + __num(b.width), y + __num(b.height)];
  }
  return null;
}

var __uiTypes = {
  window: "Window", panel: "Panel", group: "Group", button: "Button", iconbutton: "IconButton",
  statictext: "StaticText", edittext: "EditText", checkbox: "Checkbox", radiobutton: "RadioButton",
  slider: "Slider", scrollbar: "Scrollbar", progressbar: "Progressbar", dropdownlist: "DropDownList",
  listbox: "ListBox", tabbedpanel: "TabbedPanel", tab: "Tab", image: "ScriptUIImage"
};
var __uiContainers = { window: 1, panel: 1, group: 1, tabbedpanel: 1, tab: 1 };

function Window(type, title, bounds, props) {
  if (!(this instanceof Window)) return new Window(type, title, bounds, props);
  type = String(type === undefined ? "window" : type).toLowerCase();
  if (type.indexOf("{") >= 0) throw __err("ScriptUI resource strings are not supported: build the window with add()");
  if (type !== "dialog" && type !== "palette" && type !== "window") throw __err("Bad window type " + type);
  __uiInit(this, "window", null, null, bounds, title, props);
  this.__kind = type;
  this.__root = true;
  __uiWins.push(this);
}
Window.alert = function (s) { alert(s); };
Window.confirm = function (s) { return confirm(s); };
Window.prompt = function (m, d) { return prompt(m, d); };
Window.find = function (type, title) {
  for (var i = 0; i < __uiWins.length; i++) if (__uiWins[i].text === title) return __uiWins[i];
  return null;
};

function Panel() {}
function Group() {}
function Button() {}
function IconButton() {}
function StaticText() {}
function EditText() {}
function Checkbox() {}
function RadioButton() {}
function Slider() {}
function Scrollbar() {}
function Progressbar() {}
function DropDownList() {}
function ListBox() {}
function TabbedPanel() {}
function Tab() {}
function ScriptUIImage() {}
function ListItem() {}

var ScriptUI = {
  version: "6.2.2",
  frameworkName: "EffectCraft",
  environment: { keyboardState: { shiftKey: false, ctrlKey: false, altKey: false, metaKey: false, keyName: "" } },
  FontStyle: { REGULAR: 0, BOLD: 1, ITALIC: 2, BOLDITALIC: 3 },
  BrushType: { SOLID_COLOR: 0, THEME_COLOR: 1 },
  PenType: { SOLID_COLOR: 0, THEME_COLOR: 1 },
  Alignment: { LEFT: "left", RIGHT: "right", TOP: "top", BOTTOM: "bottom", CENTER: "center", FILL: "fill" },
  newFont: function (name, style, size) { return { name: name, style: style, size: size }; },
  newImage: function () { return {}; },
  getResourceText: function (t) { return t; },
  events: { createEvent: function (t) { return { type: t }; } }
};

function __uiGraphics() {
  return {
    newBrush: function (t, c) { return { type: t, color: c }; },
    newPen: function (t, c, w) { return { type: t, color: c, lineWidth: w }; },
    newPath: function () {}, moveTo: function () {}, lineTo: function () {}, rectPath: function () {},
    ellipsePath: function () {}, fillPath: function () {}, strokePath: function () {}, closePath: function () {},
    drawString: function () {}, drawOSControl: function () {}, drawImage: function () {}, drawFocusRing: function () {},
    measureString: function (s) { return __dim(String(s).length * 7, 16); },
    font: null, foregroundColor: null, backgroundColor: null, disabledForegroundColor: null, disabledBackgroundColor: null
  };
}

function __uiInit(o, type, win, parent, bounds, text, props) {
  o.type = type;
  o.window = win || o;
  o.parent = parent || null;
  o.__id = win ? win.__next++ : 0;
  if (!win) {
    o.__next = 1;
    o.__wid = __uiNewId();
  }
  o.properties = props || {};
  o.text = text === undefined || text === null ? "" : String(text);
  o.enabled = true;
  o.visible = true;
  o.helpTip = "";
  o.children = [];
  o.__ub = __toBounds(bounds);
  o.__lb = [0, 0, 0, 0];
  o.preferredSize = __dim(-1, -1);
  o.minimumSize = __dim(0, 0);
  o.maximumSize = __dim(10000, 10000);
  o.alignment = null;
  o.graphics = __uiGraphics();
  o.__listeners = {};
  if (o.properties.name) o.name = o.properties.name;
  if (__uiContainers[type]) {
    o.orientation = type === "group" ? "row" : type === "tabbedpanel" ? "stack" : "column";
    o.alignChildren = type === "group" ? ["center", "center"] : ["center", "top"];
    o.spacing = 10;
    o.margins = type === "window" ? [15, 15, 15, 15] : type === "panel" ? [10, 15, 10, 10] : type === "tab" ? [10, 10, 10, 10] : [0, 0, 0, 0];
    var self = o;
    o.layout = {
      layout: function () { __uiLayout(self.window); },
      resize: function () { __uiLayout(self.window); },
      margins: o.margins,
      spacing: o.spacing
    };
  }
  if (type === "checkbox" || type === "radiobutton") o.value = !!o.properties.value;
  if (type === "slider" || type === "scrollbar" || type === "progressbar") {
    o.value = 0;
    o.minvalue = 0;
    o.maxvalue = 100;
  }
  if (type === "edittext") {
    o.characters = 0;
    o.textselection = "";
    o.active = false;
  }
  if (type === "dropdownlist" || type === "listbox") {
    o.items = [];
    o.__sel = [];
    var it = o.properties.items;
    if (it) for (var i = 0; i < it.length; i++) o.add("item", it[i]);
  }
  if (type === "tabbedpanel") o.__sel = [];
  return o;
}

function __uiProto() {
  return {
    add: function (type, bounds, text, props) {
      var t = String(type).toLowerCase();
      if (t === "item" || t === "separator") return this.__addItem(t === "separator" ? "-" : bounds);
      if (!__uiContainers[this.type]) throw __err("add(): a " + this.type + " can't contain controls");
      var ctor = __uiTypes[t];
      if (!ctor || t === "window") throw __err("add(): unknown control type " + type);
      var c = Object.create(__ecGlobal[ctor].prototype);
      // Value arguments after `text`: slider (value, min, max), progressbar (value, max), lists (items).
      __uiInit(c, t, this.window, this, bounds, typeof text === "object" && text !== null && !(text instanceof Array) ? "" : text, props);
      if ((t === "dropdownlist" || t === "listbox") && text instanceof Array) {
        for (var i = 0; i < text.length; i++) c.add("item", text[i]);
        c.text = "";
      }
      if (t === "slider" || t === "scrollbar") {
        c.text = "";
        c.value = text === undefined ? 0 : __num(text);
        if (arguments.length > 3 && typeof props === "number") {
          c.minvalue = props;
          c.maxvalue = arguments[4] === undefined ? 100 : __num(arguments[4]);
          c.properties = arguments[5] || {};
        }
      }
      if (t === "progressbar") {
        c.text = "";
        c.value = text === undefined ? 0 : __num(text);
        if (typeof props === "number") {
          c.maxvalue = props;
          c.properties = arguments[4] || {};
        }
      }
      if (t === "checkbox" || t === "radiobutton") c.value = false;
      if (t === "edittext" && c.properties.multiline) c.__multiline = true;
      if (t === "edittext" && c.properties.readonly) c.__readonly = true;
      if (c.properties && c.properties.name) c.name = c.properties.name;
      this.children.push(c);
      if (t === "tab" && this.type === "tabbedpanel" && this.__sel.length === 0) this.__sel = [this.children.length - 1];
      return c;
    },
    remove: function (what) {
      var list = this.items && this.type !== "tabbedpanel" ? this.items : this.children;
      var i = typeof what === "number" ? what : list.indexOf(what);
      if (i < 0 || i >= list.length) return;
      list.splice(i, 1);
      if (this.items) {
        for (var k = 0; k < this.items.length; k++) this.items[k].index = k;
        this.__sel = this.__sel.filter(function (s) { return s !== i; }).map(function (s) { return s > i ? s - 1 : s; });
      }
    },
    removeAll: function () {
      if (this.items) {
        this.items = [];
        this.__sel = [];
      } else this.children = [];
    },
    find: function (text) {
      var list = this.items || this.children;
      for (var i = 0; i < list.length; i++) if (list[i].text === text) return list[i];
      return null;
    },
    __addItem: function (text) {
      if (!this.items) throw __err("add(\"item\"): only lists have items");
      var it = Object.create(ListItem.prototype);
      it.text = String(text === undefined ? "" : text);
      it.type = text === "-" ? "separator" : "item";
      it.index = this.items.length;
      it.parent = this;
      it.image = null;
      it.checked = false;
      this.items.push(it);
      return it;
    },
    show: function () {
      if (!this.__root) {
        this.visible = true;
        return;
      }
      this.__shown = true;
      this.__closed = false;
      this.__fire("onShow");
      if (this.__kind === "dialog") {
        this.__modal = true;
        var r = __uiModal(this.__wid);
        this.__modal = false;
        return r === null ? undefined : r;
      }
      return undefined;
    },
    hide: function () {
      if (this.__root) {
        this.close(2);
      } else this.visible = false;
    },
    close: function (result) {
      if (!this.__root) return;
      if (this.__fire("onClose") === false) return;
      this.__closed = true;
      this.__shown = false;
      this.__result = result === undefined ? 0 : result;
      if (this.__modal) __uiResults[this.__wid] = this.__result;
    },
    center: function () {},
    notify: function (ev) {
      var e = ev || (this.type === "edittext" || this.type === "slider" || this.type === "dropdownlist" || this.type === "listbox" ? "onChange" : "onClick");
      if (e.indexOf("on") !== 0) e = "on" + e.charAt(0).toUpperCase() + e.slice(1);
      this.__fire(e);
    },
    addEventListener: function (name, fn) {
      var k = String(name).toLowerCase();
      (this.__listeners[k] = this.__listeners[k] || []).push(fn);
    },
    removeEventListener: function (name, fn) {
      var k = String(name).toLowerCase();
      if (this.__listeners[k]) this.__listeners[k] = this.__listeners[k].filter(function (f) { return f !== fn; });
    },
    dispatchEvent: function (e) { this.__fire("on" + String(e.type || e).charAt(0).toUpperCase() + String(e.type || e).slice(1)); },
    __fire: function (name) {
      var r;
      if (typeof this[name] === "function") r = this[name].call(this);
      var l = this.__listeners[name.slice(2).toLowerCase()];
      if (l) for (var i = 0; i < l.length; i++) l[i].call(this, { type: name.slice(2).toLowerCase(), target: this });
      return r;
    },
    __handlers: function () {
      var out = [];
      var names = ["onClick", "onChange", "onChanging", "onDoubleClick", "onClose", "onShow", "onActivate", "onDeactivate", "onResize"];
      for (var i = 0; i < names.length; i++) {
        var n = names[i];
        if (typeof this[n] === "function" || (this.__listeners[n.slice(2).toLowerCase()] || []).length) out.push(n);
      }
      return out;
    },
    toString: function () { return "[object " + (__uiTypes[this.type] || "Window") + "]"; }
  };
}

(function () {
  var all = ["Window", "Panel", "Group", "Button", "IconButton", "StaticText", "EditText", "Checkbox", "RadioButton", "Slider",
    "Scrollbar", "Progressbar", "DropDownList", "ListBox", "TabbedPanel", "Tab", "ScriptUIImage"];
  for (var i = 0; i < all.length; i++) {
    var C = __ecGlobal[all[i]];
    var p = __uiProto();
    for (var k in p) C.prototype[k] = p[k];
    Object.defineProperty(C.prototype, "bounds", {
      get: function () { var b = this.__ub || this.__lb; return __bounds4(b[0], b[1], b[2], b[3]); },
      set: function (v) { this.__ub = __toBounds(v); },
      configurable: true
    });
    Object.defineProperty(C.prototype, "size", {
      get: function () { var b = this.bounds; return __dim(b[2] - b[0], b[3] - b[1]); },
      set: function (v) {
        var b = this.bounds;
        var w = v.width !== undefined ? v.width : v[0], h = v.height !== undefined ? v.height : v[1];
        this.__ub = [b[0], b[1], b[0] + __num(w), b[1] + __num(h)];
      },
      configurable: true
    });
    Object.defineProperty(C.prototype, "location", {
      get: function () { var b = this.bounds; return __dim(b[0], b[1]); },
      set: function (v) {
        var b = this.bounds;
        var x = v.x !== undefined ? v.x : v[0], y = v.y !== undefined ? v.y : v[1];
        this.__ub = [__num(x), __num(y), __num(x) + b[2] - b[0], __num(y) + b[3] - b[1]];
      },
      configurable: true
    });
  }
  var sel = {
    get: function () {
      var self = this;
      if (this.type === "tabbedpanel") return this.__sel.length ? this.children[this.__sel[0]] : null;
      var items = this.__sel.map(function (i) { return self.items[i]; });
      if (this.properties.multiselect) return items.length ? items : null;
      return items.length ? items[0] : null;
    },
    set: function (v) {
      var self = this;
      var list = this.type === "tabbedpanel" ? this.children : this.items;
      var idx = function (x) { return typeof x === "number" ? x : list.indexOf(x); };
      if (v === null || v === undefined) this.__sel = [];
      else if (v instanceof Array) this.__sel = v.map(idx).filter(function (i) { return i >= 0 && i < list.length; });
      else {
        var i = idx(v);
        this.__sel = i >= 0 && i < list.length ? [i] : [];
      }
      if (this.items) for (var k = 0; k < this.items.length; k++) this.items[k].selected = this.__sel.indexOf(k) >= 0;
    },
    configurable: true
  };
  Object.defineProperty(DropDownList.prototype, "selection", sel);
  Object.defineProperty(ListBox.prototype, "selection", sel);
  Object.defineProperty(TabbedPanel.prototype, "selection", sel);
  Object.defineProperty(ListItem.prototype, "toString", { value: function () { return this.text; } });
})();

// A dockable ScriptUI panel (scripts in the ScriptUI Panels folder run with `this` = it).
function __uiDockPanel(title) {
  var p = Object.create(Panel.prototype);
  __uiInit(p, "panel", null, null, undefined, title, {});
  p.type = "panel";
  p.__kind = "panel";
  p.__root = true;
  p.__shown = true;
  p.margins = [10, 10, 10, 10];
  __uiWins.push(p);
  return p;
}

function __uiAlign(v) {
  if (v === null || v === undefined) return [];
  if (v instanceof Array) return v.map(function (x) { return String(x).toLowerCase(); });
  return [String(v).toLowerCase()];
}
function __uiMargins(m) {
  if (m === null || m === undefined) return null;
  if (typeof m === "number") return [m, m, m, m];
  if (m.length === 4) return [m[0], m[1], m[2], m[3]];
  return null;
}
function __uiSize(d) {
  if (!d) return null;
  var w = d.width !== undefined ? d.width : d[0], h = d.height !== undefined ? d.height : d[1];
  w = Number(w);
  h = Number(h);
  if (!(w > 0) && !(h > 0)) return null;
  return [w > 0 ? w : 0, h > 0 ? h : 0];
}

function __uiJson(c) {
  var ps = __uiSize(c.preferredSize);
  if (c.type === "edittext" && c.characters > 0) ps = [c.characters * 7 + 12, ps ? ps[1] : 0];
  var o = {
    id: c.__id,
    type: c.type,
    name: String(c.name || c.properties.name || ""),
    text: String(c.text === undefined || c.text === null ? "" : c.text),
    enabled: c.enabled !== false,
    visible: c.visible !== false,
    helpTip: String(c.helpTip || ""),
    handlers: c.__handlers(),
    children: []
  };
  if (ps) o.preferredSize = ps;
  if (c.__ub && !c.__root) o.fixedBounds = c.__ub;
  if (c.__root && c.__ub) o.fixedBounds = [0, 0, c.__ub[2] - c.__ub[0], c.__ub[3] - c.__ub[1]];
  if (__uiContainers[c.type]) {
    o.orientation = String(c.orientation || "").toLowerCase();
    o.alignChildren = __uiAlign(c.alignChildren);
    var m = __uiMargins(c.margins);
    if (m) o.margins = m;
    if (typeof c.spacing === "number") o.spacing = c.spacing;
    var kids = c.children;
    for (var i = 0; i < kids.length; i++) o.children.push(__uiJson(kids[i]));
  }
  if (c.alignment !== null && c.alignment !== undefined) o.alignment = __uiAlign(c.alignment);
  if (c.type === "checkbox" || c.type === "radiobutton") o.checked = !!c.value;
  if (c.type === "slider" || c.type === "scrollbar" || c.type === "progressbar") {
    o.value = Number(c.value) || 0;
    o.min = Number(c.minvalue) || 0;
    o.max = Number(c.maxvalue);
    if (!(o.max === o.max)) o.max = 100;
  }
  if (c.items) {
    o.items = c.items.map(function (it) { return String(it.text); });
    o.selection = c.__sel.slice();
  }
  if (c.type === "tabbedpanel") o.activeTab = c.__sel.length ? c.__sel[0] : 0;
  if (c.type === "edittext") {
    o.multiline = !!c.__multiline;
    o.readOnly = !!c.__readonly;
  }
  return o;
}

// The open windows of this script, for the host.
function __uiSnapshot() {
  var out = [];
  for (var i = 0; i < __uiWins.length; i++) {
    var w = __uiWins[i];
    out.push({ id: w.__wid, kind: w.__kind, title: String(w.text || ""), visible: !!w.__shown && !w.__closed, modal: !!w.__modal, root: __uiJson(w) });
  }
  // Closed windows are forgotten.
  __uiWins = __uiWins.filter(function (w) { return !w.__closed; });
  return JSON.stringify(out);
}

function __uiOpenCount() {
  var n = 0;
  for (var i = 0; i < __uiWins.length; i++) if (__uiWins[i].__shown && !__uiWins[i].__closed) n++;
  return n;
}

function __uiWin(id) {
  for (var i = 0; i < __uiWins.length; i++) if (__uiWins[i].__wid === id) return __uiWins[i];
  return null;
}
function __uiFind(c, id) {
  if (c.__id === id) return c;
  for (var i = 0; i < (c.children || []).length; i++) {
    var r = __uiFind(c.children[i], id);
    if (r) return r;
  }
  return null;
}

// layout.layout(): lay the window out now (the host does it before drawing anyway) so scripts
// can read bounds and sizes.
function __uiLayout(win) {
  var b = JSON.parse(__uiLayoutNative(JSON.stringify({ id: win.__wid, kind: win.__kind, title: String(win.text || ""), visible: true, root: __uiJson(win) })));
  // Bounds are relative to the parent container, as in ScriptUI.
  (function apply(c, ox, oy) {
    var r = b[c.__id];
    if (!r) return;
    c.__lb = [r[0] - ox, r[1] - oy, r[0] - ox + r[2], r[1] - oy + r[3]];
    for (var i = 0; i < (c.children || []).length; i++) apply(c.children[i], r[0], r[1]);
  })(win, 0, 0);
}

// A user action from the frontend.
function __uiDispatch(winId, id, kind, valueJson) {
  var w = __uiWin(winId);
  if (!w) return;
  var v = valueJson === undefined || valueJson === "" ? null : JSON.parse(valueJson);
  if (kind === "close") {
    w.close(v === null ? 2 : v);
    return;
  }
  var c = __uiFind(w, id);
  if (!c || c.enabled === false) return;
  var radio = function () {
    var sib = c.parent ? c.parent.children : [];
    for (var i = 0; i < sib.length; i++) if (sib[i] !== c && sib[i].type === "radiobutton") sib[i].value = false;
    c.value = true;
  };
  if (kind === "click") {
    if (c.type === "checkbox") c.value = !c.value;
    else if (c.type === "radiobutton") radio();
    else if (c.type === "tab" && c.parent) {
      c.parent.selection = c;
      c.parent.__fire("onChange");
      return;
    }
    var handled = c.__handlers().indexOf("onClick") >= 0;
    c.__fire("onClick");
    // Dialog buttons without handlers: OK closes with 1, Cancel with 2.
    if (!handled && c.type === "button" && w.__kind === "dialog") {
      var n = String(c.name || c.properties.name || c.text).toLowerCase();
      if (c === w.defaultElement || n === "ok") w.close(1);
      else if (c === w.cancelElement || n === "cancel") w.close(2);
    }
    return;
  }
  // change / changing
  if (c.type === "edittext") c.text = v === null ? "" : String(v);
  else if (c.type === "statictext") c.text = v === null ? "" : String(v);
  else if (c.type === "slider" || c.type === "scrollbar" || c.type === "progressbar") c.value = Number(v);
  else if (c.type === "checkbox") c.value = !!v;
  else if (c.type === "radiobutton") { if (v) radio(); else c.value = false; }
  else if (c.type === "dropdownlist" || c.type === "listbox" || c.type === "tabbedpanel") c.selection = v;
  if (c.type === "edittext" || c.type === "slider" || c.type === "scrollbar") c.__fire("onChanging");
  if (kind === "change") {
    if (c.type === "checkbox" || c.type === "radiobutton") c.__fire("onClick");
    else c.__fire("onChange");
  }
}
