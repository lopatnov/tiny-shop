Feature: Catalog taxonomy management
  In order to organise products for buyers
  As a marketplace admin
  I want to manage the category hierarchy, attributes, and multilingual names

  Scenario: Root category appears in the catalog tree
    Given a fresh catalog
    When I create a root category "Electronics" with slug "electronics"
    Then the catalog root has 1 category
    And the first root category is named "Electronics"

  Scenario: Subcategory is accessible under its parent
    Given a fresh catalog
    And a root category "Electronics" with slug "electronics" exists
    When I create a subcategory "Phones" with slug "phones" under "Electronics"
    Then "Electronics" has 1 child category
    And the child is named "Phones"

  Scenario: Category name falls back to Ukrainian when no translation exists
    Given a fresh catalog
    And a root category "Електроніка" with slug "elektronika" exists
    When I look up the category "elektronika" in Russian
    Then the resolved name is "Електроніка"

  Scenario: Russian translation overrides the Ukrainian default
    Given a fresh catalog
    And a root category "Електроніка" with slug "elektronika" exists
    And I add a Russian translation "Электроника" for category "elektronika"
    When I look up the category "elektronika" in Russian
    Then the resolved name is "Электроника"

  Scenario: Duplicate slug within the same parent is rejected
    Given a fresh catalog
    And a root category "Electronics" with slug "electronics" exists
    When I try to create another root category with slug "electronics"
    Then the operation fails
