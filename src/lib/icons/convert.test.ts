import { describe, expect, it } from "vitest";
import { componentName, jsxAttrName, styleObject, svgToJsx, toReact, toSvelte } from "./convert";

const svg = '<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24"><g fill="none" stroke="currentColor" stroke-width="2" class="a"><path fill-rule="evenodd" d="M0 0"/></g></svg>';

describe("jsx", () => {
  it("renames attributes", () => {
    expect(jsxAttrName("stroke-width")).toBe("strokeWidth");
    expect(jsxAttrName("class")).toBe("className");
    expect(jsxAttrName("xlink:href")).toBe("xlinkHref");
    expect(jsxAttrName("aria-hidden")).toBe("aria-hidden");
    expect(jsxAttrName("data-x-y")).toBe("data-x-y");
    expect(jsxAttrName("clip-path")).toBe("clipPath");
  });

  it("converts inline style to an object", () => {
    expect(styleObject("fill-opacity: .5; --x: 1")).toBe('{{ fillOpacity: ".5", "--x": "1" }}');
  });

  it("converts markup and escapes braces in text", () => {
    const jsx = svgToJsx(svg);
    expect(jsx).toContain('strokeWidth="2"');
    expect(jsx).toContain('className="a"');
    expect(jsx).toContain('fillRule="evenodd"');
    expect(jsx).toContain("<path fillRule=\"evenodd\" d=\"M0 0\" />");
    expect(svgToJsx('<style>.a{fill:red}</style>')).toBe('<style>.a{"{"}fill:red{"}"}</style>');
  });

  it("wraps a typed react component with props spread", () => {
    const code = toReact(svg, "arrow-left");
    expect(code).toContain("export function ArrowLeft(props: SVGProps<SVGSVGElement>)");
    expect(code).toContain('viewBox="0 0 24 24" {...props}>');
  });
});

describe("svelte", () => {
  it("wraps a svelte 5 component and escapes braces", () => {
    const code = toSvelte('<svg viewBox="0 0 1 1"><style>.a{b:c}</style></svg>');
    expect(code).toContain("let props: SVGAttributes<SVGSVGElement> = $props();");
    expect(code).toContain('<svg viewBox="0 0 1 1" {...props}>');
    expect(code).toContain(".a&#123;b:c&#125;");
  });
});

describe("componentName", () => {
  it("makes pascal case names", () => {
    expect(componentName("arrow-left")).toBe("ArrowLeft");
    expect(componentName("3d-box")).toBe("Icon3dBox");
    expect(componentName("---")).toBe("Icon");
  });
});
