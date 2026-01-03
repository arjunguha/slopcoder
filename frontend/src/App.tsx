import { Router, Route } from "@solidjs/router";
import TaskList from "./components/TaskList";
import TaskDetail from "./components/TaskDetail";

function Layout(props: { children: any }) {
  return (
    <div class="min-h-screen bg-white dark:bg-gray-900 text-gray-900 dark:text-gray-100 transition-colors">
      <header class="bg-gray-100 dark:bg-gray-800 border-b border-gray-200 dark:border-gray-700">
        <div class="max-w-6xl mx-auto px-4 py-4">
          <a href="/" class="text-xl font-bold text-gray-900 dark:text-white hover:text-gray-600 dark:hover:text-gray-200">
            Slopcoder
          </a>
        </div>
      </header>
      <main class="max-w-6xl mx-auto px-4 py-8">{props.children}</main>
    </div>
  );
}

export default function App() {
  return (
    <Router>
      <Route path="/" component={() => <Layout><TaskList /></Layout>} />
      <Route path="/new" component={() => <Layout><TaskList /></Layout>} />
      <Route path="/tasks/:id" component={() => <Layout><TaskDetail /></Layout>} />
    </Router>
  );
}
