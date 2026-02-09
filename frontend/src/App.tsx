import { Router, Route } from "@solidjs/router";
import Workspace from "./components/Workspace";

export default function App() {
  return (
    <Router>
      <Route path="/" component={Workspace} />
      <Route path="/new" component={Workspace} />
      <Route path="/tasks/:id" component={Workspace} />
    </Router>
  );
}
