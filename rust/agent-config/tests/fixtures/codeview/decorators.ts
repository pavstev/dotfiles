@Component({
  selector: "app-root",
  template: "<div>{{ title }}</div>",
})
export class AppComponent {
  @Input() title = "x";
  @Output() changed = new EventEmitter();
}

@Injectable()
@Log("svc")
class Service {
  @HostListener("click")
  onClick() {
    return 1;
  }
  run() {}
}

// @deprecated
function plain() {
  return 1;
}
