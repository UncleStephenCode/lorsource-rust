from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).parents[1] / "extract_original_routes.py"
SPEC = importlib.util.spec_from_file_location("extract_original_routes", MODULE_PATH)
assert SPEC and SPEC.loader
extractor = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(extractor)


class ExtractOriginalRoutesTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tempdir = tempfile.TemporaryDirectory()
        self.root = Path(self.tempdir.name)

    def tearDown(self) -> None:
        self.tempdir.cleanup()

    def write(self, relative: str, content: str) -> None:
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")

    def test_scala_java_arrays_bare_mapping_and_metadata(self) -> None:
        self.write(
            "src/main/scala/example/Controllers.scala",
            r'''
package example

@Controller
@RequestMapping(path = Array("/people/{nick}"), method = Array(RequestMethod.GET, RequestMethod.HEAD))
class PeopleController {
  @ModelAttribute("filters")
  def filters: java.util.List[String] = java.util.List.of()

  @RequestMapping
  def profile(@PathVariable nick: String,
              @RequestParam(value = "offset", defaultValue = "0") rawOffset: Int): ModelAndView = {
    val mav = new ModelAndView("profile")
    mav.addObject("person", nick)
    mav
  }
}

class ApiController {
  @ResponseBody
  @RequestMapping(
    path = Array("/api/{id:\\d{4}}", "/legacy/{id}"),
    method = Array(RequestMethod.POST),
    params = Array("mode=full"),
    headers = Array("Accept=application/json"),
    consumes = Array("application/x-www-form-urlencoded"),
    produces = Array("application/json")
  )
  def update(@PathVariable("id") id: Int,
             @RequestParam(name = "mode", required = false) modeArg: String,
             @ModelAttribute("form") form: DemoForm,
             request: HttpServletRequest): Json = {
    request.getParameter("csrf")
    Json.obj()
  }

  @RequestMapping(path = Array("/after"), method = Array(RequestMethod.GET))
  @ResponseBody
  def after: Json = Json.obj()
}

class DemoForm(
  @BeanProperty var title: String,
  @BooleanBeanProperty var enabled: Boolean)
''',
        )
        self.write(
            "src/main/java/example/JavaBoxlet.java",
            r'''
package example;
@Controller
@RequestMapping({"/box", "/legacy-box"})
public class JavaBoxlet {
  @RequestMapping({"one.boxlet", "two.boxlet"})
  protected ModelAndView getData(@RequestParam("limit") int limit) {
    return new ModelAndView("boxlet");
  }
}
''',
        )
        self.write(
            "src/main/webapp/WEB-INF/jsp/profile.jsp",
            '<%@ page contentType="text/html; charset=utf-8" %><html></html>',
        )

        routes = extractor.extract_routes(self.root)
        self.assertEqual(8, len(routes))

        inherited = next(row for row in routes if row["handler"] == "profile")
        self.assertEqual(["GET", "HEAD"], inherited["methods"])
        self.assertEqual(["ANY"], inherited["declared_methods"])
        self.assertTrue(inherited["mapping_is_bare"])
        self.assertEqual("0", inherited["request_params"][0]["default"])
        self.assertFalse(inherited["request_params"][0]["required"])
        self.assertEqual(["person"], inherited["model_keys"])
        self.assertEqual(["profile"], inherited["view_names"])
        self.assertEqual("text/html; charset=utf-8", inherited["response_content_types"][0]["value"])
        self.assertEqual("filters", inherited["controller_model_attributes"][0]["name"])
        self.assertEqual("filters", inherited["controller_model_attributes"][0]["provider"])

        constrained = next(row for row in routes if row["spring_path"] == r"/api/{id:\\d{4}}")
        self.assertEqual("/api/{id}", constrained["path"])
        self.assertTrue(constrained["path_has_constraints"])
        self.assertEqual(["Accept=application/json"], constrained["headers"])
        self.assertEqual(["application/x-www-form-urlencoded"], constrained["consumes"])
        self.assertEqual(["application/json"], constrained["produces"])
        self.assertTrue(constrained["response_body"])
        self.assertEqual("json", constrained["response_kind"])
        self.assertEqual(["enabled", "title"], constrained["form_fields"])
        self.assertEqual({"mode", "csrf"}, {item["name"] for item in constrained["request_params"]})
        self.assertTrue(next(row for row in routes if row["path"] == "/after")["response_body"])

        java_rows = [row for row in routes if row["source_language"] == "java"]
        self.assertEqual(
            {"/box/one.boxlet", "/box/two.boxlet", "/legacy-box/one.boxlet", "/legacy-box/two.boxlet"},
            {row["path"] for row in java_rows},
        )
        self.assertTrue(all(row["handler"] == "getData" for row in java_rows))

    def test_class_and_method_path_arrays_expand_cartesian_product(self) -> None:
        self.write(
            "src/main/scala/example/ArrayController.scala",
            '''
@Controller
@RequestMapping(value = Array("/v1", "/v2"), params = Array("tenant"))
class ArrayController {
  @RequestMapping(path = Array("/a", "/b"), params = Array("mode"))
  def list: ModelAndView = new ModelAndView("list")
}
''',
        )
        routes = extractor.extract_routes(self.root)
        self.assertEqual({"/v1/a", "/v1/b", "/v2/a", "/v2/b"}, {row["path"] for row in routes})
        self.assertTrue(all(row["params"] == ["tenant", "mode"] for row in routes))

    def test_response_status_does_not_leak_to_the_next_mapping(self) -> None:
        self.write(
            "src/main/scala/example/StatusController.scala",
            '''
@Controller
@RequestMapping(Array("/items"))
class StatusController {
  @RequestMapping(params = Array("output=rss"))
  @ResponseStatus(HttpStatus.GONE)
  def gone: ModelAndView = new ModelAndView("errors/code410")

  @RequestMapping
  def normal: ModelAndView = new ModelAndView("items")

  @ExceptionHandler(Array(classOf[ItemNotFoundException]))
  @ResponseStatus(HttpStatus.NOT_FOUND)
  def notFound: ModelAndView = new ModelAndView("errors/code404")
}
''',
        )
        routes = {row["handler"]: row for row in extractor.extract_routes(self.root)}
        self.assertEqual("GONE", routes["gone"]["response_status"])
        self.assertIsNone(routes["normal"]["response_status"])
        self.assertEqual("NOT_FOUND", routes["normal"]["controller_exception_handlers"][0]["response_status"])
        self.assertEqual(["ItemNotFoundException"], routes["normal"]["controller_exception_handlers"][0]["exceptions"])

    def test_surface_includes_websocket_rewrite_resources_servlets_and_static(self) -> None:
        self.write(
            "src/main/scala/example/Realtime.scala",
            '''
class Config extends WebSocketConfigurer {
  def registerWebSocketHandlers(registry: WebSocketHandlerRegistry): Unit = {
    registry.addHandler(handler, "/ws").setAllowedOrigins(config.getSecureUrl)
    registry.addResourceHandler("/images/*.png", "/images/*.gif")
      .addResourceLocations("file:/srv/images/")
  }
  val one = TextMessage.Strict(s"comment $comment")
  val two = TextMessage.Strict("events-refresh")
}
''',
        )
        self.write(
            "src/main/java/example/BinderAdvice.java",
            '''
@ControllerAdvice
@Order(10)
public class BinderAdvice {
  @InitBinder
  public void bind(WebDataBinder binder) {
    String[] denylist = new String[]{"class.", ".class."};
    binder.setDisallowedFields(denylist);
  }
}
''',
        )
        self.write(
            "src/main/webapp/WEB-INF/urlrewrite.xml",
            '''<?xml version="1.0"?>
<urlrewrite>
  <rule><from>^/old$</from><to type="redirect">/new</to></rule>
  <outbound-rule><from>^(.*)$</from><to>$1</to></outbound-rule>
</urlrewrite>
''',
        )
        self.write(
            "src/main/webapp/WEB-INF/springapp-servlet.xml",
            '''<beans xmlns:mvc="urn:mvc">
  <mvc:default-servlet-handler/>
  <mvc:resources mapping="/webjars/**" location="classpath:/webjars/" cache-period="10"/>
  <mvc:interceptors><bean class="example.SecurityInterceptor"/></mvc:interceptors>
</beans>
''',
        )
        self.write(
            "src/main/webapp/WEB-INF/web.xml",
            '''<web-app>
  <filter-mapping><filter-name>security</filter-name><url-pattern>/*</url-pattern></filter-mapping>
  <error-page><error-code>404</error-code><location>/errors/404</location></error-page>
  <session-config><session-timeout>60</session-timeout></session-config>
  <servlet><multipart-config><max-file-size>100</max-file-size></multipart-config></servlet>
  <servlet-mapping><servlet-name>app</servlet-name><url-pattern>/</url-pattern></servlet-mapping>
</web-app>
''',
        )
        self.write("src/main/webapp/img/logo.png", "not an actual image")
        self.write("src/main/webapp/robots.txt", "User-agent: *")

        surface = extractor.extract_original_surface(self.root)
        self.assertEqual(["/ws"], [row["path"] for row in surface["websocket"]])
        self.assertEqual("config.getSecureUrl", surface["websocket"][0]["allowed_origins_expression"])
        self.assertEqual(2, len(surface["url_rewrite"]))
        self.assertEqual(
            {"/images/*.png", "/images/*.gif", "/webjars/**"},
            {row["path"] for row in surface["resource_handlers"]},
        )
        self.assertEqual(["/"], [row["path"] for row in surface["servlet_mappings"]])
        self.assertEqual(["/*"], [row["path"] for row in surface["filter_mappings"]])
        self.assertEqual("/errors/404", surface["error_pages"][0]["path"])
        self.assertEqual("example.SecurityInterceptor", surface["interceptors"][0]["class"])
        self.assertEqual("60", surface["webapp_settings"]["session_timeout_minutes"])
        self.assertEqual("100", surface["webapp_settings"]["multipart"]["max_file_size"])
        self.assertEqual(["class.", ".class."], surface["controller_advice"][0]["disallowed_binding_fields"])
        static = {row["path"]: row for row in surface["static_surface"]}
        self.assertEqual(1, static["/img/**"]["file_count"])
        self.assertEqual("default servlet", static["/robots.txt"]["served_by"])


if __name__ == "__main__":
    unittest.main()
